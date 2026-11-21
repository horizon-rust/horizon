# Permission nodes

A permission node is a named, typed question a server mod asks about a player's UUID, for example whether a
player may harvest a rare crop. Nodes are server-side only. Each node has a default answer, and that default
answers every query unless the server selects another handler. Nodes are separate from operator levels: a node
grant does not give a player an operator level, a node denial does not remove one, and vanilla's own commands
and checks do not consult nodes.

## Declaring nodes

Add each node in the `permission_gather_nodes` hook through its collector, `PermissionNodes::add(node, default)`.
Gathering runs once per logical server start, after operator levels load and before the server starts. There is
no late registration: a node added after that is refused.

- Identity is the node's case-sensitive dot-separated name plus its value type. Boolean, integer, string and
  text-component nodes are distinct nodes even under one name.
- Adding the same typed node twice is a registration error, and no partial service is published.
- A Boolean node's default is the operator level that grants it by default (`NodeDefault::Level`). Other types
  take a default value of their own type (`NodeDefault::Value`). A Boolean node declared with
  `NodeDefault::Nobody` is granted to no one by default, operators included (Bukkit's `default: false`): it
  answers `false` until a handler grants it. `Nobody` on a node of another type is refused with `WrongType`. A default is data, not a function: there is no
  mod-written resolver yet, so a policy that depends on the player or on contexts belongs in a handler.
- The built-in handler reads the player's operator level when the query runs: an online player's live level, else
  the dedicated server's operator list by UUID, else level 0. It ignores the contexts. A caller may leave out
  any context key; the service refuses only too many contexts or an empty or over-long key.

## Handlers

Exactly one handler answers every node on a server. The built-in default handler answers from each node's
default. A mod can provide its own handler with `permission_gather_handler`, which pushes the handler's id. The server
then asks that mod through `permission_handler_query` for each query. The hook is called on the providing mod
alone (its `provider` argument): no other mod is called, so none sees the player's UUID or the contexts. No answer,
an answer from another mod or an answer of the wrong type is an error. Duplicate or empty handler ids are
registration errors, and the server then answers no permission query.

The dedicated server's `permission-handler` setting names the handler to use. With no setting (or `default`) the
default handler is used. An id no mod provided is logged by name and startup does not fail, but it is not the
default handler: a handler was configured and none could be selected, so the default handler still answers
`permission_query`, and command usages that ask a node are refused (see "Commands that use a node").

A handler can answer independently of operator levels. Its failures are never a grant: an error, a trap or a
recursive query is an explicit `PermissionError`, and the protected action must treat it as a refusal. Integer,
string and component errors must not be read as a Boolean denial.

## Querying

| Call | Behavior |
|---|---|
| `permission_query(node, uuid, contexts)` | Returns `Result<PermissionValue, PermissionError>`: the node's typed value from the selected handler. |
| `permission_bool(player, &node)` | The same for a Boolean node and a connected player, as `Result<bool, PermissionError>`. |

`PermissionError` names each refusal: `NotInitialized` (a query before the service is ready), `Unregistered`
(an unknown node, never an implicit registration), `DuplicateNode`, `DuplicateHandler`, `WrongType`,
`UnknownPlayer`, `HandlerFailed`, `Recursive` (a handler that queries inside its own query fails both queries)
and `InvalidInput`. In `permission_handler_query`, `online` is the player's UUID when the player is on this
server and `None` for an offline UUID; ask `host.player(uuid)` for the handle. `default` is the `NodeDefault` the node was declared with, so a handler with no rule of its own for a node
can answer from it: a `Level` against the player's operator level, a `Value` as it is. A failed query changes nothing and grants nothing.

Queries take no cache and write nothing. A node decision does not fire `permissions_changed`, and that hook
does not report node decisions. There are no wildcards, no hierarchy and no namespace ownership: a node name
means exactly what it spells.

Put the check at the server-side point where the action happens, and refuse the action on any error.

## Commands that use a node

A mod command path may declare a Boolean node: push `Command::new(usage).with_node(&node)` in
`register_node_commands`. `register_commands` keeps its API 0.2 payload (a `Command` has only a usage and a
level), so a mod built for 0.2 that registers commands loads as before; both hooks fill one command tree. A usage
with a node is decided in this order:

1. The configured handler's answer decides.
2. With the built-in handler (no `permission-handler`, or `default`), the usage's operator level decides, as in
   vanilla.
3. A handler error denies the path and is logged. It never falls back to the operator level.
4. When the permission service could not be set up (two mods declared one node or one handler id), or
   `permission-handler` names a handler no mod provides (a typo, or the handler mod missing or disabled), the
   usage is left out of the command tree and logged. It never runs at an operator level. A mod's own
   `permission_query` calls are not changed by this.

A path whose check fails is left out of the command tree sent to that player (its nodes are not in the packet at
all) and of the completions the server answers them, and typing it answers as an unknown command. An argument the
server cannot parse on such a path still answers with that parse error, because the line is parsed before its node
is asked. Each node is asked once per player when a tree is
sent, however many paths it gates. Commands without a node keep their operator level, and vanilla's commands are unchanged.

## Refreshing a player's command tree

`host.player_resend_command_tree(player)` sends one player their command tree again, as vanilla does on join,
respawn, dimension change and an operator change, with the mods' say on it. Call it when a handler's answers for
that player change. Only that player's client is sent a packet. Called from inside `player_commands_sent` it does
nothing, as the pass that asked that hook sends the trees when it returns. The server checks every command run anyway, so a
stale tree affects only the suggestions the client shows.

It returns `Result<(), PlayerWorldRefusal>`, refusing with `Client` on a client, `Stale` for a player that is not
in the server's list (or a handle of an earlier open of the world) and `NoConnection` for a fake player. A
refusal sends nothing.

A handler that reads `host.player_permission_level` while a clickable sign or a function runs a command sees the
level that run has (2 for a sign, as in vanilla, where only an operator can write one), not the player's own. A
handler that needs the player's own level must not rely on it during sign or function commands.

## What a node does not do

- A node does not change `/op`, the operator list, or any vanilla command or command-block check.
- A node is not a cache: each query asks the handler again.
- A handler cannot be chained with another handler; one answers each node.
