---
name: no_db_wipe
description: NEVER wipe the SQLite database when deploying updates to stake_watch
type: feedback
---

NEVER run `sudo rm -f /opt/stake-watch/data/stake_watch.db*` during deploys.

**Why:** The user's watch list, stake history, and alert subscriptions are in that DB. Wiping it forces them to re-register everything. Schema changes should be handled with migrations (ALTER TABLE or CREATE TABLE IF NOT EXISTS), not by deleting the database.

**How to apply:** Deploy = stop service, copy binary + static files, start service. Nothing else. If a schema change requires migration, add an ALTER TABLE in the db init code.
