# Docker Compose Setup

This directory contains Docker Compose configurations for local testing.

## Files to be created:

- `docker-compose.postgresql.yml` - PostgreSQL + observability stack
- `docker-compose.tigerbeetle.yml` - TigerBeetle + observability stack
- `Dockerfile.client` - Client binary container (if needed)

## Services:

Each compose file should include:
- Database (PostgreSQL or TigerBeetle)
- OpenTelemetry Collector
- Prometheus
- Grafana

## Usage:

```bash
# Start PostgreSQL environment
docker-compose -f docker-compose.postgresql.yml up -d

# Start TigerBeetle environment
docker-compose -f docker-compose.tigerbeetle.yml up -d
```
