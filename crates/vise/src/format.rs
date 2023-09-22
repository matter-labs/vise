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
    use crate::{Counter, Gauge, LabeledFamily, Metrics, Registry, Unit};

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

    #[derive(Debug, Metrics)]
    #[metrics(crate = crate, prefix = "test")]
    pub(crate) struct TestMetrics {
        /// Test counter.
        counter: Counter,
        #[metrics(labels = ["method"])]
        family_of_counters: LabeledFamily<&'static str, Counter>,
        #[metrics(unit = Unit::Bytes)]
        gauge: Gauge<usize>,
        #[metrics(labels = ["code"])]
        family_of_gauges: LabeledFamily<u16, Gauge>,
    }

    #[test]
    fn translating_real_metrics() {
        let test_metrics = TestMetrics::default();
        let mut registry = Registry::empty();
        registry.register_metrics(&test_metrics);

        test_metrics.counter.inc_by(3);
        test_metrics.family_of_counters[&"call"].inc_by(5);
        test_metrics.family_of_counters[&"send_tx"].inc_by(2);
        test_metrics.gauge.set(42);
        test_metrics.family_of_gauges[&200].set(12);
        test_metrics.family_of_gauges[&404].set(4);

        let common_expected_lines = [
            "# HELP test_counter Test counter.",
            "# TYPE test_counter counter",
            "# TYPE test_family_of_counters counter",
            "# TYPE test_gauge_bytes gauge",
            "# UNIT test_gauge_bytes bytes",
            "test_gauge_bytes 42",
            "# TYPE test_family_of_gauges gauge",
            "test_family_of_gauges{code=\"200\"} 12",
            "test_family_of_gauges{code=\"404\"} 4",
        ];
        let om_expected_lines = [
            "test_counter_total 3",
            "test_family_of_counters_total{method=\"call\"} 5",
            "test_family_of_counters_total{method=\"send_tx\"} 2",
        ];
        let prom_expected_lines = [
            "test_counter 3",
            "test_family_of_counters{method=\"call\"} 5",
            "test_family_of_counters{method=\"send_tx\"} 2",
        ];
        let eof = "# EOF";

        assert_format(
            &registry,
            Format::OpenMetrics,
            common_expected_lines
                .into_iter()
                .chain(om_expected_lines)
                .chain([eof]),
            prom_expected_lines.into_iter(),
        );

        for prom_format in [Format::OpenMetricsForPrometheus, Format::Prometheus] {
            let expect_eof = matches!(prom_format, Format::OpenMetricsForPrometheus);
            let expected_lines = common_expected_lines
                .into_iter()
                .chain(prom_expected_lines)
                .chain(expect_eof.then_some(eof));
            let missing_lines = om_expected_lines
                .into_iter()
                .chain((!expect_eof).then_some(eof));
            assert_format(&registry, prom_format, expected_lines, missing_lines);
        }
    }

    fn assert_format<'a>(
        registry: &Registry,
        format: Format,
        expected_lines: impl Iterator<Item = &'a str>,
        missing_lines: impl Iterator<Item = &'a str>,
    ) {
        println!("Testing {format:?}");
        let mut buffer = String::new();
        registry.encode(&mut buffer, format).unwrap();
        let lines: Vec<_> = buffer.lines().collect();
        for expected_line in expected_lines {
            assert!(
                lines.contains(&expected_line),
                "Line `{expected_line}` is missing: {lines:#?}"
            );
        }
        for missing_line in missing_lines {
            assert!(
                !lines.contains(&missing_line),
                "Line `{missing_line}` is present: {lines:#?}"
            );
        }
    }
}
