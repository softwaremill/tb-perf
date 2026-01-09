# Database Setup Scripts

Scripts for database initialization and management.

## Scripts to create:

### PostgreSQL
- `init-postgresql.sh` - Create schema, stored procedures, initial accounts
- `setup-replication.sh` - Configure PostgreSQL 3-node replication
- `reset-postgresql.sh` - DROP DATABASE and reinitialize

### TigerBeetle
- `init-tigerbeetle.sh` - Initialize TigerBeetle cluster, create accounts
- `setup-cluster.sh` - Configure 3-node TigerBeetle cluster
- `reset-tigerbeetle.sh` - Truncate transfers, reset balances

### Shared
- `verify-balances.sh` - Check sum of all account balances for correctness validation
