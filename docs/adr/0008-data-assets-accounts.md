# 0008: Persistence, assets, accounts and campaigns

Status: Accepted (2026-11-12)

## Decision

- **Database:** PostgreSQL holds accounts, campaigns, the event log and snapshots. Live scene state is in the server's memory and written behind.
- **Assets:** maps, token art and other uploads go to S3-compatible object storage (Cloudflare R2 in production, MinIO locally) and are served through a CDN. The database stores references and metadata only.
- **Accounts:** email and password (Argon2 hashing), plus OAuth with Discord and Google. The class demo uses email and password only; OAuth follows.
- **Campaigns:** a GM creates a campaign and shares an invite link. People sign in and join. Roles are GM, player and spectator; the GM can promote a co-GM. The role field is an open enum so more roles can be added without a breaking change.
- **Permissions** are checked on the server for every intent.
- **Hosting:** Docker Compose on the home Ubuntu server behind a Cloudflare Tunnel, front end on Cloudflare Pages, on a subdomain to be chosen.

## Why

- Memory-first state keeps play fast; write-behind keeps it durable.
- Large maps served from a home connection would be the bottleneck for every player, so they go to object storage.
- This matches how Creature Cauldron is already deployed.

## Rejected

- **SQLite:** fine for one machine, harder to grow.
- **No accounts:** no ownership or saved characters.
- **Assets on local disk or in the database:** bandwidth and backup problems.
- **A public campaign browser:** moderation burden.
