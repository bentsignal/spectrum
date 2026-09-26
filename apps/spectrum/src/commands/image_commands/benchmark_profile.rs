use super::*;

#[derive(Clone, Copy, Default, ValueEnum)]
pub(super) enum BenchmarkProfile {
    #[default]
    Interactive,
    HostedCi,
}

impl BenchmarkProfile {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Interactive => "interactive-workstation",
            Self::HostedCi => "github-hosted-linux",
        }
    }

    pub(super) fn preview_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 50.0,
            // The shared two-core Linux runner measured 97 ms p95 at the
            // workstation-passing baseline. Keep 29% headroom for host jitter.
            Self::HostedCi => 125.0,
        }
    }

    pub(super) fn live_preview_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 33.0,
            Self::HostedCi => 85.0,
        }
    }

    pub(super) fn command_budget_ms(self) -> f64 {
        match self {
            // A durable command atomically republishes a portable project that
            // contains a 24 MP source asset. It runs after interaction preview,
            // so this gate protects completion latency rather than frame time.
            Self::Interactive => 100.0,
            Self::HostedCi => 175.0,
        }
    }

    pub(super) fn switch_dispatch_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 4.0,
            Self::HostedCi => 12.0,
        }
    }

    pub(super) fn prefetched_switch_ready_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 35.0,
            Self::HostedCi => 75.0,
        }
    }

    pub(super) fn cold_switch_ready_budget_ms(self) -> f64 {
        match self {
            // The deterministic 2400x1600 JPEG path measured 120 ms p95
            // locally. Keep 46% workstation and 150% hosted-runner headroom
            // without allowing a regression into visibly sluggish switching.
            Self::Interactive => 175.0,
            Self::HostedCi => 300.0,
        }
    }

    pub(super) fn requires_incremental_publication(self) -> bool {
        matches!(self, Self::HostedCi)
    }
}
