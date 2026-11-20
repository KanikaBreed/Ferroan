# Collector adapter

Ferroan posts OTLP/HTTP JSON to `[Monitoring] OtlpEndpoint` when that key is set. It does not speak to Loki, OpenSearch, SigNoz, Graylog or Splunk itself. This directory is the adapter: pick a file and give it to the OpenTelemetry Collector. The servers stay as they are.

`loki.yaml` is the compose example. It listens on `127.0.0.1:4318` (HTTP) and `127.0.0.1:4317` (gRPC), reads `*.log` under `/var/log/ferroan` (mount the container log files there), and forwards logs to Loki at `http://127.0.0.1:3100/otlp`. Traces and metrics go to the collector's debug exporter. Loki stores logs, so a login span is also copied into a log line `trace span=login trace_id=...` by Ferroan. That line is what shows up in Loki next to `login accepted`.

The image is `otel/opentelemetry-collector-contrib:0.128.0`. The core collector image has no Loki exporter.

| File | System |
| --- | --- |
| `loki.yaml` | Loki, with Grafana reading `http://127.0.0.1:3100` |
| `opensearch.yaml` | OpenSearch `http://opensearch:9200`, index `ferroan-logs` |
| `signoz.yaml` | SigNoz OTLP/HTTP `http://signoz:4318` |
| `graylog.yaml` | Graylog's OTLP/gRPC input `graylog:4317` |
| `splunk.yaml` | Splunk HEC `https://splunk:8088/services/collector` |

`otelcol validate` checks the four files that smoke does not start. Only Loki runs in `cargo xtask docker smoke`.

The contrib collector has no GELF exporter. `graylog.yaml` uses Graylog's OTLP input, not a GELF payload. A GELF-only Graylog needs an input this file does not configure.

To try another system, change the collector command's config path in `deploy/compose/compose.yaml` from `loki.yaml` to the file you want. Replace host names and the Splunk token. Do not put that token in the compose file.
