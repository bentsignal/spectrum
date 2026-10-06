use clap::ValueEnum;

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

    pub(super) fn edit_budget_ms(self) -> f64 {
        match self {
            // A saved edit reopens the image's document, which embeds a
            // 24 MP source, and publishes one revision. It runs after the
            // preview, so this protects completion latency, not frame time.
            Self::Interactive => 100.0,
            Self::HostedCi => 175.0,
        }
    }
}
