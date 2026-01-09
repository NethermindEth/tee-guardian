# Guardian Test Observability

OpenObserve is used as the single observability platform for Guardian tests,
providing logs, metrics, and traces in one unified interface.

## Access

- **URL**: http://localhost:5080
- **Username**: root@example.com
- **Password**: Complexpass#123

## Features

- **Logs**: Guardian nodes send logs via HTTP to OpenObserve's log ingestion API
- **Metrics**: Prometheus-compatible scraping from Guardian `/metrics` endpoints
- **Traces**: OpenTelemetry-compatible trace ingestion (if enabled)

## Prometheus Scrape Configuration

Metrics are scraped from Guardian VMs at `192.168.122.{20-119}:8443/metrics`.
The scrape configuration is managed dynamically based on which clusters are active.

## Log Shipping

Guardian VMs are configured to ship logs to OpenObserve via the 
HTTP endpoint at `http://172.20.0.2:5080/api/default/guardian_logs/_json`.
