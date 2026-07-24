use clap::ValueEnum;

#[derive(Clone, Copy, Default, ValueEnum)]
pub(crate) enum BenchmarkProfile {
    #[default]
    Interactive,
    HostedCi,
}

impl BenchmarkProfile {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Interactive => "interactive-workstation",
            Self::HostedCi => "github-hosted-linux",
        }
    }

    pub(crate) fn gradient_shadow_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 500.0,
            // The reviewed implementation measured 880.788 ms p95 on GitHub's
            // shared Linux runner versus 222.508 ms locally. A 1,250 ms ceiling
            // keeps 42% host-jitter headroom while the original 2,061.886 ms
            // regression still fails decisively.
            Self::HostedCi => 1_250.0,
        }
    }

    pub(crate) fn magic_wand_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5_000.0,
            Self::HostedCi => 15_000.0,
        }
    }

    pub(crate) fn raster_delete_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 75.0,
            Self::HostedCi => 225.0,
        }
    }

    pub(crate) fn near_cap_raster_delete_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5_000.0,
            Self::HostedCi => 15_000.0,
        }
    }

    pub(crate) fn path_raster_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 250.0,
            Self::HostedCi => 750.0,
        }
    }

    pub(crate) fn path_edit_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 5.0,
            Self::HostedCi => 15.0,
        }
    }

    pub(crate) fn paint_viewport_budget_ms(self) -> f64 {
        match self {
            Self::Interactive => 500.0,
            Self::HostedCi => 1_500.0,
        }
    }
}
