use std::fmt::Write;

use super::diagnostics::RuntimeSnapshot;

pub(crate) const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Format a [`RuntimeSnapshot`] as Prometheus text exposition format.
pub(crate) fn format_prometheus(snapshot: &RuntimeSnapshot) -> String {
    let mut out = String::with_capacity(2048);

    // Bootstrap status (gauge)
    write_gauge(
        &mut out,
        "foundry_bootstrap_complete",
        "Whether bootstrap has completed",
        if snapshot.bootstrap_complete { 1 } else { 0 },
    );

    // HTTP request counters
    write_help_type(
        &mut out,
        "foundry_http_requests_total",
        "Total HTTP requests handled",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_http_requests_total",
        "class",
        "1xx",
        snapshot.http.informational_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_requests_total",
        "class",
        "2xx",
        snapshot.http.success_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_requests_total",
        "class",
        "3xx",
        snapshot.http.redirection_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_requests_total",
        "class",
        "4xx",
        snapshot.http.client_error_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_requests_total",
        "class",
        "5xx",
        snapshot.http.server_error_total,
    );
    write_help_type(
        &mut out,
        "foundry_http_edge_rejections_total",
        "Total HTTP edge rejections by reason",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_http_edge_rejections_total",
        "reason",
        "rate_limited",
        snapshot.http.edge_rejections.rate_limited_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_edge_rejections_total",
        "reason",
        "payload_too_large",
        snapshot.http.edge_rejections.payload_too_large_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_edge_rejections_total",
        "reason",
        "timeout",
        snapshot.http.edge_rejections.timeout_total,
    );
    write_counter_label(
        &mut out,
        "foundry_http_edge_rejections_total",
        "reason",
        "cors",
        snapshot.http.edge_rejections.cors_rejected_total,
    );
    write_help_type(
        &mut out,
        "foundry_http_request_duration_ms",
        "HTTP request duration histogram in milliseconds",
        "histogram",
    );
    for bucket in &snapshot.http.duration_ms.buckets {
        write_labeled_sample(
            &mut out,
            "foundry_http_request_duration_ms_bucket",
            &[("le", &bucket.le_ms.to_string())],
            bucket.cumulative_count,
        );
    }
    write_labeled_sample(
        &mut out,
        "foundry_http_request_duration_ms_bucket",
        &[("le", "+Inf")],
        snapshot.http.duration_ms.count,
    );
    write_sample(
        &mut out,
        "foundry_http_request_duration_ms_sum",
        snapshot.http.duration_ms.sum_ms,
    );
    write_sample(
        &mut out,
        "foundry_http_request_duration_ms_count",
        snapshot.http.duration_ms.count,
    );

    // Auth counters
    write_help_type(
        &mut out,
        "foundry_auth_total",
        "Total authentication outcomes",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_auth_total",
        "outcome",
        "success",
        snapshot.auth.success_total,
    );
    write_counter_label(
        &mut out,
        "foundry_auth_total",
        "outcome",
        "unauthorized",
        snapshot.auth.unauthorized_total,
    );
    write_counter_label(
        &mut out,
        "foundry_auth_total",
        "outcome",
        "forbidden",
        snapshot.auth.forbidden_total,
    );
    write_counter_label(
        &mut out,
        "foundry_auth_total",
        "outcome",
        "error",
        snapshot.auth.error_total,
    );

    // WebSocket counters
    write_help_type(
        &mut out,
        "foundry_websocket_connections_total",
        "Total WebSocket connections opened",
        "counter",
    );
    write_sample(
        &mut out,
        "foundry_websocket_connections_total",
        snapshot.websocket.opened_total,
    );
    write_help_type(
        &mut out,
        "foundry_websocket_connection_events_total",
        "Total WebSocket connection lifecycle events",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_websocket_connection_events_total",
        "state",
        "opened",
        snapshot.websocket.opened_total,
    );
    write_counter_label(
        &mut out,
        "foundry_websocket_connection_events_total",
        "state",
        "closed",
        snapshot.websocket.closed_total,
    );
    write_gauge(
        &mut out,
        "foundry_websocket_active_connections",
        "Currently active WebSocket connections",
        snapshot.websocket.active_connections,
    );
    write_help_type(
        &mut out,
        "foundry_websocket_subscription_events_total",
        "Total WebSocket subscription lifecycle events",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_websocket_subscription_events_total",
        "action",
        "subscribe",
        snapshot.websocket.subscriptions_total,
    );
    write_counter_label(
        &mut out,
        "foundry_websocket_subscription_events_total",
        "action",
        "unsubscribe",
        snapshot.websocket.unsubscribes_total,
    );
    write_gauge(
        &mut out,
        "foundry_websocket_active_subscriptions_global",
        "Currently active WebSocket subscriptions across all channels",
        snapshot.websocket.active_subscriptions,
    );

    write_help_type(
        &mut out,
        "foundry_websocket_messages_total",
        "Total WebSocket messages",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_websocket_messages_total",
        "direction",
        "inbound",
        snapshot.websocket.inbound_messages_total,
    );
    write_counter_label(
        &mut out,
        "foundry_websocket_messages_total",
        "direction",
        "outbound",
        snapshot.websocket.outbound_messages_total,
    );

    // Per-channel WebSocket series
    write_help_type(
        &mut out,
        "foundry_websocket_subscriptions_total",
        "Total WebSocket subscriptions per channel",
        "counter",
    );
    write_help_type(
        &mut out,
        "foundry_websocket_active_subscriptions",
        "Currently active WebSocket subscriptions per channel",
        "gauge",
    );
    write_help_type(
        &mut out,
        "foundry_websocket_channel_messages_total",
        "Total WebSocket messages per channel",
        "counter",
    );
    write_help_type(
        &mut out,
        "foundry_websocket_channel_unsubscribes_total",
        "Total WebSocket unsubscribes per channel",
        "counter",
    );
    for channel in &snapshot.websocket.channels {
        write_labeled_sample(
            &mut out,
            "foundry_websocket_subscriptions_total",
            &[("channel", channel.id.as_str())],
            channel.subscriptions_total,
        );
        write_labeled_sample(
            &mut out,
            "foundry_websocket_channel_unsubscribes_total",
            &[("channel", channel.id.as_str())],
            channel.unsubscribes_total,
        );
        write_labeled_sample(
            &mut out,
            "foundry_websocket_active_subscriptions",
            &[("channel", channel.id.as_str())],
            channel.active_subscriptions,
        );
        write_labeled_sample(
            &mut out,
            "foundry_websocket_channel_messages_total",
            &[("channel", channel.id.as_str()), ("direction", "inbound")],
            channel.inbound_messages_total,
        );
        write_labeled_sample(
            &mut out,
            "foundry_websocket_channel_messages_total",
            &[("channel", channel.id.as_str()), ("direction", "outbound")],
            channel.outbound_messages_total,
        );
    }

    // Scheduler counters
    write_help_type(
        &mut out,
        "foundry_scheduler_ticks_total",
        "Total scheduler ticks",
        "counter",
    );
    write_sample(
        &mut out,
        "foundry_scheduler_ticks_total",
        snapshot.scheduler.ticks_total,
    );
    write_help_type(
        &mut out,
        "foundry_scheduler_executions_total",
        "Total scheduled tasks executed",
        "counter",
    );
    write_sample(
        &mut out,
        "foundry_scheduler_executions_total",
        snapshot.scheduler.executed_schedules_total,
    );
    write_gauge(
        &mut out,
        "foundry_scheduler_leader_active",
        "Whether this instance is the active scheduler leader",
        if snapshot.scheduler.leader_active {
            1
        } else {
            0
        },
    );
    write_help_type(
        &mut out,
        "foundry_scheduler_leadership_total",
        "Total scheduler leadership changes",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_scheduler_leadership_total",
        "state",
        "acquired",
        snapshot.scheduler.leadership_acquired_total,
    );
    write_counter_label(
        &mut out,
        "foundry_scheduler_leadership_total",
        "state",
        "lost",
        snapshot.scheduler.leadership_lost_total,
    );

    // Job counters
    write_help_type(
        &mut out,
        "foundry_jobs_total",
        "Total job lifecycle events",
        "counter",
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "enqueued",
        snapshot.jobs.enqueued_total,
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "leased",
        snapshot.jobs.leased_total,
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "started",
        snapshot.jobs.started_total,
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "succeeded",
        snapshot.jobs.succeeded_total,
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "retried",
        snapshot.jobs.retried_total,
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "expired_lease_requeued",
        snapshot.jobs.expired_requeues_total,
    );
    write_counter_label(
        &mut out,
        "foundry_jobs_total",
        "outcome",
        "dead_lettered",
        snapshot.jobs.dead_lettered_total,
    );

    out
}

fn write_help_type(out: &mut String, name: &str, help: &str, metric_type: &str) {
    let escaped_help = escape_prometheus_help_text(help);
    let _ = writeln!(out, "# HELP {name} {escaped_help}");
    let _ = writeln!(out, "# TYPE {name} {metric_type}");
}

fn write_gauge(out: &mut String, name: &str, help: &str, value: u64) {
    write_help_type(out, name, help, "gauge");
    write_sample(out, name, value);
}

fn write_counter_label(out: &mut String, name: &str, label: &str, label_value: &str, value: u64) {
    write_labeled_sample(out, name, &[(label, label_value)], value);
}

fn write_sample(out: &mut String, name: &str, value: u64) {
    let _ = writeln!(out, "{name} {value}");
}

fn write_labeled_sample(out: &mut String, name: &str, labels: &[(&str, &str)], value: u64) {
    let _ = write!(out, "{name}{{");
    for (index, (label, label_value)) in labels.iter().enumerate() {
        if index > 0 {
            let _ = write!(out, ",");
        }
        let escaped = escape_prometheus_label_value(label_value);
        let _ = write!(out, "{label}=\"{escaped}\"");
    }
    let _ = writeln!(out, "}} {value}");
}

fn escape_prometheus_help_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str(r"\\"),
            '\n' => escaped.push_str(r"\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_prometheus_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str(r"\\"),
            '"' => escaped.push_str(r#"\""#),
            '\n' => escaped.push_str(r"\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::diagnostics::{
        AuthRuntimeSnapshot, HttpDurationBucketSnapshot, HttpDurationHistogramSnapshot,
        HttpEdgeRejectionSnapshot, HttpRuntimeSnapshot, JobRuntimeSnapshot, RuntimeSnapshot,
        SchedulerRuntimeSnapshot, WebSocketRuntimeSnapshot,
    };
    use crate::logging::types::RuntimeBackendKind;

    #[test]
    fn formats_prometheus_text() {
        let snapshot = RuntimeSnapshot {
            backend: RuntimeBackendKind::Memory,
            bootstrap_complete: true,
            http: HttpRuntimeSnapshot {
                requests_total: 100,
                informational_total: 0,
                success_total: 80,
                redirection_total: 5,
                client_error_total: 10,
                server_error_total: 5,
                edge_rejections: HttpEdgeRejectionSnapshot {
                    rate_limited_total: 3,
                    payload_too_large_total: 2,
                    timeout_total: 1,
                    cors_rejected_total: 0,
                },
                duration_ms: HttpDurationHistogramSnapshot {
                    count: 100,
                    sum_ms: 12_345,
                    buckets: vec![
                        HttpDurationBucketSnapshot {
                            le_ms: 5,
                            cumulative_count: 3,
                        },
                        HttpDurationBucketSnapshot {
                            le_ms: 10,
                            cumulative_count: 9,
                        },
                        HttpDurationBucketSnapshot {
                            le_ms: 25,
                            cumulative_count: 25,
                        },
                    ],
                },
            },
            auth: AuthRuntimeSnapshot {
                success_total: 50,
                unauthorized_total: 3,
                forbidden_total: 1,
                error_total: 0,
            },
            websocket: WebSocketRuntimeSnapshot {
                opened_total: 10,
                closed_total: 5,
                active_connections: 5,
                subscriptions_total: 20,
                unsubscribes_total: 10,
                active_subscriptions: 10,
                inbound_messages_total: 100,
                outbound_messages_total: 200,
                channels: Vec::new(),
            },
            scheduler: SchedulerRuntimeSnapshot {
                ticks_total: 500,
                executed_schedules_total: 42,
                leadership_acquired_total: 2,
                leadership_lost_total: 1,
                leader_active: true,
            },
            jobs: JobRuntimeSnapshot {
                enqueued_total: 30,
                leased_total: 28,
                started_total: 28,
                succeeded_total: 25,
                retried_total: 2,
                expired_requeues_total: 1,
                dead_lettered_total: 0,
            },
        };

        let output = format_prometheus(&snapshot);

        assert!(output.contains("foundry_bootstrap_complete 1"));
        assert!(output.contains("foundry_http_requests_total{class=\"2xx\"} 80"));
        assert!(output.contains("foundry_http_requests_total{class=\"5xx\"} 5"));
        assert!(output.contains("foundry_http_edge_rejections_total{reason=\"rate_limited\"} 3"));
        assert!(
            output.contains("foundry_http_edge_rejections_total{reason=\"payload_too_large\"} 2")
        );
        assert!(output.contains("foundry_http_edge_rejections_total{reason=\"timeout\"} 1"));
        assert!(output.contains("# TYPE foundry_http_request_duration_ms histogram"));
        assert!(output.contains("foundry_http_request_duration_ms_bucket{le=\"25\"} 25"));
        assert!(output.contains("foundry_http_request_duration_ms_bucket{le=\"+Inf\"} 100"));
        assert!(output.contains("foundry_http_request_duration_ms_sum 12345"));
        assert!(output.contains("foundry_http_request_duration_ms_count 100"));
        assert!(output.contains("foundry_auth_total{outcome=\"success\"} 50"));
        assert!(output.contains("foundry_websocket_connections_total 10"));
        assert!(output.contains("foundry_websocket_connection_events_total{state=\"opened\"} 10"));
        assert!(output.contains("foundry_websocket_connection_events_total{state=\"closed\"} 5"));
        assert!(output.contains("foundry_websocket_active_connections 5"));
        assert!(
            output.contains("foundry_websocket_subscription_events_total{action=\"subscribe\"} 20")
        );
        assert!(output
            .contains("foundry_websocket_subscription_events_total{action=\"unsubscribe\"} 10"));
        assert!(output.contains("foundry_websocket_active_subscriptions_global 10"));
        assert!(output.contains("foundry_jobs_total{outcome=\"succeeded\"} 25"));
        assert!(output.contains("foundry_jobs_total{outcome=\"leased\"} 28"));
        assert!(output.contains("foundry_jobs_total{outcome=\"expired_lease_requeued\"} 1"));
        assert!(output.contains("foundry_scheduler_leader_active 1"));
        assert!(output.contains("foundry_scheduler_leadership_total{state=\"acquired\"} 2"));
        assert!(output.contains("foundry_scheduler_leadership_total{state=\"lost\"} 1"));
        assert!(output.contains("# TYPE foundry_http_requests_total counter"));
        assert!(output.contains("# TYPE foundry_bootstrap_complete gauge"));
        assert!(output.ends_with('\n'));
    }

    #[test]
    fn prometheus_helpers_escape_help_text() {
        let mut output = String::new();

        write_help_type(
            &mut output,
            "foundry_test_total",
            "Line one\\line two\nline three",
            "counter",
        );

        assert_eq!(
            output,
            "# HELP foundry_test_total Line one\\\\line two\\nline three\n# TYPE foundry_test_total counter\n"
        );
    }

    #[test]
    fn format_prometheus_emits_per_channel_websocket_series() {
        use crate::logging::diagnostics::WebSocketChannelSnapshot;
        use crate::support::ChannelId;

        let snapshot = RuntimeSnapshot {
            backend: RuntimeBackendKind::Memory,
            bootstrap_complete: false,
            http: HttpRuntimeSnapshot {
                requests_total: 0,
                informational_total: 0,
                success_total: 0,
                redirection_total: 0,
                client_error_total: 0,
                server_error_total: 0,
                edge_rejections: HttpEdgeRejectionSnapshot::default(),
                duration_ms: HttpDurationHistogramSnapshot {
                    count: 0,
                    sum_ms: 0,
                    buckets: Vec::new(),
                },
            },
            auth: AuthRuntimeSnapshot {
                success_total: 0,
                unauthorized_total: 0,
                forbidden_total: 0,
                error_total: 0,
            },
            websocket: WebSocketRuntimeSnapshot {
                opened_total: 0,
                closed_total: 0,
                active_connections: 5,
                subscriptions_total: 0,
                unsubscribes_total: 0,
                active_subscriptions: 0,
                inbound_messages_total: 0,
                outbound_messages_total: 0,
                channels: vec![WebSocketChannelSnapshot {
                    id: ChannelId::new("chat"),
                    subscriptions_total: 10,
                    unsubscribes_total: 2,
                    active_subscriptions: 8,
                    inbound_messages_total: 100,
                    outbound_messages_total: 300,
                }],
            },
            scheduler: SchedulerRuntimeSnapshot {
                ticks_total: 0,
                executed_schedules_total: 0,
                leadership_acquired_total: 0,
                leadership_lost_total: 0,
                leader_active: false,
            },
            jobs: JobRuntimeSnapshot {
                enqueued_total: 0,
                leased_total: 0,
                started_total: 0,
                succeeded_total: 0,
                retried_total: 0,
                expired_requeues_total: 0,
                dead_lettered_total: 0,
            },
        };

        let output = format_prometheus(&snapshot);

        assert!(output.contains("foundry_websocket_active_connections 5"));
        assert!(
            output.contains("foundry_websocket_subscriptions_total{channel=\"chat\"} 10"),
            "missing per-channel subscriptions series:\n{output}"
        );
        assert!(output.contains("foundry_websocket_channel_unsubscribes_total{channel=\"chat\"} 2"));
        assert!(output.contains("foundry_websocket_active_subscriptions{channel=\"chat\"} 8"));
        assert!(output.contains(
            "foundry_websocket_channel_messages_total{channel=\"chat\",direction=\"inbound\"} 100"
        ));
        assert!(output.contains(
            "foundry_websocket_channel_messages_total{channel=\"chat\",direction=\"outbound\"} 300"
        ));
    }

    #[test]
    fn format_prometheus_escapes_label_values() {
        use crate::logging::diagnostics::WebSocketChannelSnapshot;
        use crate::support::ChannelId;

        let snapshot = RuntimeSnapshot {
            backend: RuntimeBackendKind::Memory,
            bootstrap_complete: false,
            http: HttpRuntimeSnapshot {
                requests_total: 0,
                informational_total: 0,
                success_total: 0,
                redirection_total: 0,
                client_error_total: 0,
                server_error_total: 0,
                edge_rejections: HttpEdgeRejectionSnapshot::default(),
                duration_ms: HttpDurationHistogramSnapshot {
                    count: 0,
                    sum_ms: 0,
                    buckets: Vec::new(),
                },
            },
            auth: AuthRuntimeSnapshot {
                success_total: 0,
                unauthorized_total: 0,
                forbidden_total: 0,
                error_total: 0,
            },
            websocket: WebSocketRuntimeSnapshot {
                opened_total: 0,
                closed_total: 0,
                active_connections: 0,
                subscriptions_total: 0,
                unsubscribes_total: 0,
                active_subscriptions: 0,
                inbound_messages_total: 0,
                outbound_messages_total: 0,
                channels: vec![WebSocketChannelSnapshot {
                    id: ChannelId::owned("team\"ops\\prod\nblue"),
                    subscriptions_total: 7,
                    unsubscribes_total: 0,
                    active_subscriptions: 0,
                    inbound_messages_total: 0,
                    outbound_messages_total: 0,
                }],
            },
            scheduler: SchedulerRuntimeSnapshot {
                ticks_total: 0,
                executed_schedules_total: 0,
                leadership_acquired_total: 0,
                leadership_lost_total: 0,
                leader_active: false,
            },
            jobs: JobRuntimeSnapshot {
                enqueued_total: 0,
                leased_total: 0,
                started_total: 0,
                succeeded_total: 0,
                retried_total: 0,
                expired_requeues_total: 0,
                dead_lettered_total: 0,
            },
        };

        let output = format_prometheus(&snapshot);

        assert!(output.contains(
            "foundry_websocket_subscriptions_total{channel=\"team\\\"ops\\\\prod\\nblue\"} 7"
        ));
        assert!(!output.contains("team\"ops\\prod\nblue"));
    }
}
