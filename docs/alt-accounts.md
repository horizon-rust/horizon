# Alternative accounts: Ely.by and other Yggdrasil providers

Status: **not built**. Tracked as roadmap `alt-accounts` ("Ely.by and other sign-in providers",
`later`) and issue [#1270](https://github.com/horizon-rust/horizon/issues/1270). This note is the
plan to review before code: what a third-party sign-in changes in the launcher and the game, and
the decisions that are not an agent's to make.

The Microsoft flow it builds beside is [microsoft-account.md](microsoft-account.md); offline names,
their skins and the owner gate are described there too.

## What a player would get

- **Settings → Account → Add account** offers a provider beside Microsoft: Ely.by first, then any
  Yggdrasil-compatible server (Blessing Skin, Drasl, LittleSkin, a server's own authlib).
- The account chip lists the account like a Microsoft one, labelled with its provider, and Play
  starts the game with that account's name, UUID, skin and cape.
- Servers that accept the provider — Ely.by's authlib, authlib-injector, Paper's Alternative
  Authentication — see the player as signed in, with their provider skin.
- Online-mode *Mojang* servers keep refusing it, as they refuse any account Mojang did not issue.
  The Microsoft owner gate and its download rule are unchanged.

## What this is not

- Not a way onto Mojang online-mode servers, and not a way to make Minecraft's own files appear
  without an owner (`download_for_owner` stays Microsoft-only).
- Not a password store: a Yggdrasil sign-in keeps only the access token and client token; a
  password lives in memory for the length of the request.
- Not authlib-injector: Horizon's game speaks the protocol itself, so there is no Java agent, no
  patched authlib and no JVM for Horizon instances.
- Not for Java instances (`java_player`): Mojang's game needs Mojang's token.

## Providers and identity

`Account` (`launcher/src/account.rs`) grows a provider:

```rust
enum Provider {
    /// The default, and what accounts.json held before providers existed.
    Microsoft,
    /// A Yggdrasil-compatible origin, without the trailing slash: https://authserver.ely.by.
    Yggdrasil(String),
}
```

`Provider::Microsoft` is the serde default, so existing `accounts.json` files and keychain items
keep working. Accounts are keyed by `(provider, uuid)`: two providers can issue the same UUID (an
Ely.by UUID derived from the name, a Microsoft profile), and the keychain item's name must say
which. `Account.name` stays the display name; `selected` becomes the provider-scoped key.

`Secrets` becomes one of:

- Microsoft: today's refresh token, Minecraft token and expiry, xuid, `owns` (unchanged).
- Yggdrasil: `access_token`, `client_token`, the profile UUID, and the last validation time. No
  password, no `owns` — a third-party account cannot own Java Edition.

## Signing in

Ely.by documents two ways, and recommends the first:

1. **OAuth 2.0** (`account.ely.by`), scopes `account_info minecraft_server_session offline_access`:
   the browser signs in (2FA included), a loopback redirect returns an authorization code, the
   launcher exchanges it for an access token with the `minecraft_server_session` right, and keeps
   the refresh token. This needs an OAuth application registered by the project (a client ID owned
   by Ludovic, like `HORIZON_MSA_CLIENT_ID`), and confirmation that Ely.by accepts a public client
   with PKCE and a loopback redirect — no client secret can ship in a desktop binary.
2. **Yggdrasil** (`https://authserver.ely.by/auth/authenticate`, also the generic path every
   compatible server implements): `username` (name or e-mail), `password` (with `:TOTP` appended
   when the answer is 401 "Account protected with two factor auth."), a launcher-generated
   `clientToken`, `requestUser: true`. Then `/auth/refresh` before expiry, `/auth/validate` to
   check, `/auth/invalidate` on sign-out, `/auth/signout` only if the player asks to end every
   session.

M1 implements (2): it is one client for every Yggdrasil provider, and it is honest about the
password staying in memory. (1) can be added for Ely.by later without changing the account model.

Errors map to the launcher's existing sign-in screen: invalid credentials, 2FA required, e-mail not
verified, server unreachable, and any `errorMessage` the provider sends (shown as text, never
parsed as markup).

## Playing on a server

The launcher writes the session for the game today
(`account::write_session_file`) as
`{name, uuid, access_token, user_type: "msa", xuid}`; the game reads it once
(`horizon-client-render/src/auth.rs`) and calls Mojang's session service from the discovery
document (`join_server`).

For a Yggdrasil account the file gains the provider and its session URL, and the game POSTs the
same authlib body (`accessToken`, `selectedProfile`, `serverId`) to the provider's
`POST /session/join` instead of Mojang's. Ely.by uses exactly that path. No discovery document is
involved: the origin is the account's provider, and the paths are the Yggdrasil ones.

This is the one change outside the launcher's workspace: `AuthSession` must carry the provider,
and `join_server` must choose its endpoint from it. `verify` (the server's `hasJoined`) is the
mirror image and stays Mojang-only until a server-side setting exists (below).

## Skins and capes

A provider's profile returns the same `textures` property shape Mojang uses, base64 JSON with
`SKIN`/`CAPE` entries. `horizon-client-render/src/skins.rs` already decodes it
(`unpackTextures`, `Textures`, `Signature`); what changes:

- `TEXTURE_URIS` allows only `textures.minecraft.net`. A provider's own origin must be allowed,
  per provider, and a texture URL outside it refused as today.
- Ely.by signs textures with its own key; the docs also say a signature of `Cg==` means "not
  verifiable, do not check". `Signature` gains a per-provider answer rather than Mojang's key.
- The launcher already keeps a local skin and cape per offline name (`launcher/src/skins.rs`).
  The same file picker can override a Yggdrasil account's skin for players whose provider has none,
  and is the fallback if texture loading fails.

## The owner gate

Today Play needs a Microsoft account that owns Java Edition, or the player's **Play without a
Microsoft account** (`Settings::offline_play`, off by default; see
[microsoft-account.md](microsoft-account.md#the-owner-gate)).

A Yggdrasil account cannot own Java Edition, so it must not silently stand in for an owner: that
would turn the gate into a formality. The choice is Ludovic's on #1270; the M1 recommendation is:

- a Yggdrasil account is a **named identity**, like an offline name with a skin, and Play uses it
  exactly when `offline_play` is on (or a Microsoft owner is signed in);
- it never unlocks Mojang downloads or Java instances.

Option B, if the maintainer prefers: a provider account passes Play for servers that accept that
provider, with no setting, because it is a real sign-in — but then the gate's sentence "needs an
account that owns Minecraft" has to be reworded, and the default behaviour changes.

## Horizon's own servers

`horizon-server`'s `online_mode` verifies players at Mojang
(`crates/horizon-server-files/src/profiles.rs`, `hasJoined`). Accepting Ely.by players on a Horizon
server means a `session_server` (or `online_mode = "elyby"`) setting, hasJoined against that
origin, and whitelist name→UUID lookups there too. Separate PR, after the client works; hosting
docs and `server.toml.example` change with it.

## Files

| Area | File |
|---|---|
| Account model, secrets, session file | `launcher/src/account.rs`, `launcher/src/account/tests.rs` |
| Yggdrasil client (new) | `launcher/src/account/yggdrasil.rs` |
| Add-account UI, provider label | `launcher/src/main.rs`, `launcher/src/switch_page.rs`, `launcher/src/owner_page.rs` |
| CLI and MCP | `launcher/src/cli.rs` (`account add --provider`, `account list`), `launcher/src/mcp.rs` |
| Launch and settings | `launcher/src/backend.rs` |
| Game session and join | `crates/horizon-client-render/src/auth.rs` |
| Textures | `crates/horizon-client-render/src/skins.rs` |
| Translations | `launcher/lang/{fr,ru,de}.json` (the i18n test fails on a missing string) |
| Server side (later) | `crates/horizon-server-files/src/profiles.rs`, `crates/horizon-server/src/config.rs` |

## Security

- Password only in memory, for the request; never in `accounts.json`, logs, the command line or an
  error message. Tokens go in the keychain like Microsoft's, under the provider-scoped key.
- Origins are pinned: Ely.by by default. A custom Yggdrasil origin is a player's explicit choice
  with a warning (it receives the password), never something a server address or a mod can set.
- HTTPS only; redirects not followed for token requests; responses size-capped.
- Sign-out invalidates the token with the provider (`/auth/invalidate`) and forgets it locally even
  if the network fails.

## Tests

- Unit tests against a local fake Yggdrasil server (the same way `Endpoints` are injected today):
  authenticate, 2FA challenge, refresh near expiry, validate, invalidate, provider errors, a
  texture property with and without a signature.
- Account-store tests: old `accounts.json` loads as Microsoft, two providers with one UUID stay
  apart, removing one account never touches the other's secrets.
- The session file and `join_server` endpoint choice, with a fake session service.
- CLI/MCP: `account add --provider elyby --username …` refuses without a keychain? (no: it uses the
  memory store under `HORIZON_NO_KEYCHAIN`), `account list` marks the provider, Play passes or
  refuses per the gate decision.

## Milestones

1. **M1 — sign in and play offline-mode servers** (the bulk): provider model, Yggdrasil client, UI,
   CLI/MCP, local skins fallback, tests. Launcher workspace plus the session-file change.
2. **M2 — online servers that accept the provider**: texture hosts/signatures, `join_server` per
   provider, a real Ely.by end-to-end check.
3. **M3 — Horizon-hosted servers** accept provider players (`session_server`), docs and tests.

## Open questions for #1270

1. OAuth (needs the project's Ely.by application and PKCE confirmation) or Yggdrasil
   username/password for M1?
2. Gate policy: provider accounts behind `offline_play` (recommended), or their own pass?
3. Custom provider origins: pinned list, or any origin with a warning?
4. Who owns the Ely.by OAuth registration, and the translations for the new strings?
