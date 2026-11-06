# A public address (prototype)

> **Status: a prototype for discussion** (`crates/horizon-gateway`). Nothing in the game or the launcher uses it
> yet, Horizon runs no gateway, and none is configured by default. Which shape it should take in the game is an
> open question on its issue.

Some players want what a server address gives: `pullmon.mc.example.net`, typed into **any** Minecraft client's
server list, for a world running on a PC at home, with no port forwarding. [Play with Friends](play-with-friends.md)
reaches the same people with invite codes, end-to-end encrypted, but guests need Horizon. A gateway is the
other trade: a frontend someone runs on a machine with a public address, under a wildcard domain they own.

```
 player (any 26.3 client)                gateway (a VPS, a Pi with a forwarded port)              host's PC
 ┌──────────────┐  pullmon.mc.example.net:25565  ┌────────────────────────────┐  control: dialled out  ┌──────────────┐
 │ Minecraft    │───────────────────────────────▶│ reads the handshake's      │◀───────────────────────│ agent        │
 └──────────────┘                                 │ address → "pullmon" → asks │══ a new connection ═══▶│ (expose)     │──▶ 127.0.0.1:25565
                                                  │ that host for a connection │   per player           └──────────────┘    (LAN world or
                                                  └────────────────────────────┘                                            horizon-server)
```

The host only makes outbound connections, so it works behind CGNAT, a campus network or a router with UPnP off. The
gateway reads one packet, the client's handshake (`ClientIntentionPacket`: protocol, the address the player typed,
port, status or login), routes by its first DNS label, then copies bytes unread. The host's server gets that
handshake first, unchanged, as if the player had dialled it.

## Privacy: who sees what

- **The gateway's operator** sees every player's IP address, the name of the world they join, the player's name
  (the login start packet is plain on every server) and, **in offline mode, the whole game**: chat, positions, the
  world. In **online mode** the client and the host's server encrypt everything after login end to end (vanilla's
  encryption request), so the gateway sees only the handshake, the name and the size of encrypted traffic: the
  client tells Mojang's session server a hash of the host's public key and the host checks that hash, so a gateway
  that swapped in its own key would fail it.
- **The host's server** sees every player arrive from `127.0.0.1` (the agent). IP bans and per-address limits on
  the server no longer tell players apart; the gateway has its own per-address limits.
- **Players** never learn the host's address. The host never learns players' addresses.
- Compared with a relay for invite codes, a gateway is a trusted party. Run your own, or use one run by someone you
  trust.

## Running a gateway

1. A machine with a public address (a VPS, or a home server with TCP 25565 and the control port forwarded).
2. DNS: a wildcard **A/AAAA** record, `*.mc.example.net → <that address>`. Not an SRV record: vanilla's client puts
   the SRV *target* in the login handshake (`ConnectScreen` resolves it first), so every name behind one SRV
   target would look alike.
3. Run it:

```
cargo build --release -p horizon-gateway
GATEWAY_DOMAIN=mc.example.net ./target/release/horizon-gateway serve
```

`curl http://<address>:7001/health` answers counts.

| Variable | Default | |
| --- | --- | --- |
| `GATEWAY_DOMAIN` | required | the wildcard domain players' addresses end in |
| `GATEWAY_MINECRAFT_PORT` | 25565 | where players connect |
| `GATEWAY_PUBLIC_PORT` | the Minecraft port | the port shown in addresses (behind a port mapping) |
| `GATEWAY_CONTROL_PORT` | 7001 | where hosts register and attach (also `GET /health`) |
| `GATEWAY_BIND` | all addresses | listen on one address only |
| `GATEWAY_HOST_KEYS` | unset: open | a file of admission keys, one per line: only hosts with one may register |
| `GATEWAY_MAX_HOSTS` | 500 | names registered at once |
| `GATEWAY_MAX_BRIDGES` | 200 | players connected at once |
| `GATEWAY_SESSION_KBPS` | 1024 | KiB/s each way per player |
| `GATEWAY_TOTAL_MBPS` | 100 | Mbit/s for everything bridged |
| `GATEWAY_NAME_KEPT_HOURS` | 24 | how long a host that left keeps its name |
| `GATEWAY_PER_ADDRESS_LIMITS` | 1 | per-address rate limits |

Fixed limits, from `horizon-relay`'s: per address (IPv6 by /64) 6 registrations a minute (burst 3), 60 player
connections a minute (burst 20; a server list refresh is one), 5 unknown names in 10 minutes block that address
for 10 minutes; the handshake (and a host's first line) within 10 s; a stream idle for 120 s is closed; hosts
answer pings within 60 s.

**What players see when no host answers:** the server list shows "No world has this address" or "This world is
offline" (a name whose host left, still reserved), and a login is refused with the same text.

## Putting a world behind it

```
horizon-gateway expose gateway.example.net:7001 pullmon --to 127.0.0.1:25565
Players join: pullmon.mc.example.net
To keep this name after a restart: --claim 3f0c…
```

`--to` is the server: a world opened to LAN, or `horizon-server`'s port. The agent reconnects with its claim when
the gateway drops it. A name is 3 to 32 of `a-z 0-9 -`, first come first served while its host is connected, and
then kept for `GATEWAY_NAME_KEPT_HOURS` for the claim that took it. `--key` (or `HORIZON_GATEWAY_KEY`) passes an
admission key.

## The control protocol

One TCP port, one JSON object per line until a connection is bridged (the relay's framing, `horizon_friends::wire`).

| From | Line | Meaning |
| --- | --- | --- |
| host | `{"op":"host","v":1,"name":"pullmon","claim":C?,"key":K?}` | register; answer `{"op":"registered","name":…,"address":"pullmon.mc.example.net","token":T,"claim":C}` |
| gateway → host | `{"op":"join","session":S}` | a player is joining: open a new connection and attach |
| host | `{"op":"attach","v":1,"session":S,"token":T}` | answer `{"op":"bridged","session":S}`, then the player's bytes from its handshake on |
| gateway ↔ host | `{"op":"ping"}` / `{"op":"pong"}` | every 20 s |

Errors: `{"op":"error","code":…}` with `bad_name`, `name_taken`, `unauthorized`, `full`, `rate_limited`,
`bad_request`.

## Not done

- The game and the launcher don't start an agent (a "Public address" row beside Play with Friends' code, or
  `horizon-server`'s `server.toml`): that waits for the shape the project wants.
- Names live in the gateway's memory: a restart frees them.
- The host's server can't learn players' real addresses (PROXY protocol or Velocity-style forwarding would carry
  them).
- No Dockerfile or service file yet (the relay's would fit as they are).

## Testing

- Unit tests: the handshake (a 26.3 status handshake byte for byte, legacy pings, HTTP, oversized and truncated
  frames), routing (case, trailing dot, Forge's `\0FML…` marker, nested labels, look-alike domains), names, the
  gateway's own status and disconnect answers.
- `crates/horizon-gateway/tests/gateway.rs`: two hosts behind one gateway, each player reaching the host its
  address names with the exact handshake and 1 MiB both ways; unknown and offline names; claims (taken while
  connected and after, retaken with the claim, a reconnect replacing a dead connection); admission keys; garbage
  and trickled openings; `/health`.
- By hand, 2026-11-01: a Horizon dedicated server (`horizon-server`) behind a gateway on loopback, domain
  `mc.localhost`; a vanilla-protocol server list ping of `pullmon.mc.localhost:25590` returned the server's own
  status, an unknown name the gateway's; a Horizon client joined `pullmon.mc.localhost:25590` and chatted both ways.
