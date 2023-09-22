//! Support for various metrics encoding formats.

use std::{fmt, mem};

/// Metrics export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// [OpenMetrics text format][om]. This is the original format produced by [`prometheus-client`], which
    /// this library is based upon.
    ///
    /// [om]: https://github.com/OpenObservability/OpenMetrics/blob/main/specification/OpenMetrics.md
    /// [`prometheus-client`]: https://docs.rs/prometheus-client/
    OpenMetrics,
    /// [Prometheus text format][prom]. Since it's quite similar to the OpenMetrics format, it's obtained by
    /// a streaming transform of OpenMetrics-encoded metrics that removes `_total` suffixes from
    /// reported counter values and removes the `# EOF` terminator.
    ///
    /// [prom]: https://prometheus.io/docs/instrumenting/exposition_formats/
    Prometheus,
    /// OpenMetrics text format as understood by Prometheus.
    ///
    /// Prometheus *mostly* understands the OpenMetrics format (e.g., enforcing no empty lines for it).
    /// The notable exception is counter definitions; OpenMetrics requires
    /// to append `_total` to the counter name (like `_sum` / `_count` / `_bucket` are appended
    /// to histogram names), but Prometheus doesn't understand this (yet?).
    ///
    /// See also: [issue in `prometheus-client`](https://github.com/prometheus/client_rust/issues/111)
    OpenMetricsForPrometheus,
}

#[derive(Debug)]
struct MetricTypeDefinition {
    name: String,
    is_counter: bool,
}

impl MetricTypeDefinition {
    fn parse(line: &str) -> Result<Self, fmt::Error> {
        let (name, ty) = line
            .trim()
            .split_once(|ch: char| ch.is_ascii_whitespace())
            .ok_or(fmt::Error)?;
        Ok(Self {
            name: name.to_owned(),
            is_counter: ty == "counter",
        })
    }
}

#[must_use = "Must be `flush()`ed to not lose the last line"]
#[derive(Debug)]
pub(crate) struct PrometheusWrapper<'a, W> {
    writer: &'a mut W,
    remove_eof_terminator: bool,
    last_metric_definition: Option<MetricTypeDefinition>,
    last_line: String,
}

impl<'a, W: fmt::Write> PrometheusWrapper<'a, W> {
    pub(crate) fn new(writer: &'a mut W, remove_eof_terminator: bool) -> Self {
        Self {
            writer,
            remove_eof_terminator,
            last_metric_definition: None,
            last_line: String::new(),
        }
    }

    fn handle_line(&mut self) -> fmt::Result {
        let line = mem::take(&mut self.last_line);
        if line == "# EOF" && self.remove_eof_terminator {
            // Prometheus format doesn't specify the termination sequence, so we skip it.
            return Ok(());
        }
        let mut transformed_line = None;

        if let Some(type_def) = line.strip_prefix("# TYPE ") {
            self.last_metric_definition = Some(MetricTypeDefinition::parse(type_def)?);
        } else if !line.starts_with('#') {
            // `line` reports a metric value
            let name_end_pos = line
                .find(|ch: char| ch == '{' || ch.is_ascii_whitespace())
                .ok_or(fmt::Error)?;
            let (name, rest) = line.split_at(name_end_pos);

            if let Some(metric_type) = &self.last_metric_definition {
                let truncated_name = name.strip_suffix("_total");

                if truncated_name == Some(&metric_type.name) && metric_type.is_counter {
                    // Remove `_total` suffix to the metric name, which is not present
                    // in the Prometheus text format, but is mandatory for the OpenMetrics format.
                    transformed_line = Some(format!("{}{rest}", metric_type.name));
                }
            }
        }

        let transformed_line = transformed_line.unwrap_or(line);
        writeln!(self.writer, "{transformed_line}")
    }

    pub(crate) fn flush(mut self) -> fmt::Result {
        if self.last_line.is_empty() {
            Ok(())
        } else {
            self.handle_line()
        }
    }
}

impl<W: fmt::Write> fmt::Write for PrometheusWrapper<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let lines: Vec<_> = s.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            self.last_line.push_str(line);
            if i + 1 < lines.len() || s.ends_with('\n') {
                self.handle_line()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn translating_open_metrics_format() {
        let mut buffer = String::new();
        let mut wrapper = PrometheusWrapper::new(&mut buffer, true);

        // Emulating breaking line into multiple write instructions
        write!(wrapper, "# HELP ").unwrap();
        write!(wrapper, "http_requests").unwrap();
        write!(wrapper, " ").unwrap();
        writeln!(wrapper, "Number of HTTP requests.").unwrap();

        write!(wrapper, "# TYPE ").unwrap();
        write!(wrapper, "http_requests").unwrap();
        write!(wrapper, " ").unwrap();
        writeln!(wrapper, "counter").unwrap();

        write!(wrapper, "http_requests").unwrap();
        write!(wrapper, "_total").unwrap();
        write!(wrapper, "{{").unwrap();
        write!(wrapper, "method=\"").unwrap();
        write!(wrapper, "call").unwrap();
        write!(wrapper, "\"}} ").unwrap();
        writeln!(wrapper, "42").unwrap();

        writeln!(wrapper, "# EOF").unwrap();

        wrapper.flush().unwrap();
        let lines: Vec<_> = buffer.lines().collect();
        assert_eq!(
            lines,
            [
                "# HELP http_requests Number of HTTP requests.",
                "# TYPE http_requests counter",
                "http_requests{method=\"call\"} 42",
            ]
        );
    }

    #[test]
    fn translating_sample() {
        let input = "\
            # TYPE modern_counter counter\n\
            modern_counter_total 1\n\
            # TYPE modern_gauge gauge\n\
            modern_gauge{label=\"value\"} 23\n\
            # TYPE modern_counter_with_labels counter\n\
            modern_counter_with_labels_total{label=\"value\"} 3\n\
            modern_counter_with_labels_total{label=\"other\"} 5";
        let expected = "\
            # TYPE modern_counter counter\n\
            modern_counter 1\n\
            # TYPE modern_gauge gauge\n\
            modern_gauge{label=\"value\"} 23\n\
            # TYPE modern_counter_with_labels counter\n\
            modern_counter_with_labels{label=\"value\"} 3\n\
            modern_counter_with_labels{label=\"other\"} 5\n";

        let mut buffer = String::new();
        let mut wrapper = PrometheusWrapper::new(&mut buffer, true);
        wrapper.write_str(input).unwrap();
        wrapper.flush().unwrap();

        assert_eq!(buffer, expected);
    }

    // FIXME: test defining real metrics
}
