//! Internal metrics for the exporter itself.

use std::{fmt, time::Duration};

use vise::{Buckets, EncodeLabelSet, EncodeLabelValue, Family, Global, Histogram, Metrics, Unit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue, EncodeLabelSet)]
#[metrics(label = "facade")]
pub(crate) enum Facade {
    Metrics,
    Vise,
}

impl fmt::Display for Facade {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Metrics => "metrics",
            Self::Vise => "vise",
        })
    }
}

const BYTE_BUCKETS: Buckets = Buckets::exponential(1_024.0..=1_024.0 * 1_024.0, 4.0);

#[derive(Debug, Metrics)]
#[metrics(prefix = "vise_exporter")]
pub(crate) struct ExporterMetrics {
    /// Scraping latency of the exporter.
    #[metrics(buckets = Buckets::LATENCIES, unit = Unit::Seconds)]
    pub scrape_latency: Family<Facade, Histogram<Duration>>,
    /// Size of all metrics using a certain façade.
    #[metrics(buckets = BYTE_BUCKETS, unit = Unit::Bytes)]
    pub scraped_size: Family<Facade, Histogram<usize>>,
}

// Due to the recursive nature of the metrics definition, using a collector is problematic.
#[vise::register]
pub(crate) static EXPORTER_METRICS: Global<ExporterMetrics> = Global::new();
