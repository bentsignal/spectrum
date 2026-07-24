#[cfg(target_os = "linux")]
use std::cell::Cell;

#[derive(Debug, Default)]
pub(super) struct PublishCapabilities {
    #[cfg(target_os = "linux")]
    exchange_supported: Cell<Option<bool>>,
    #[cfg(all(test, target_os = "linux"))]
    exchange_probes: Cell<u64>,
}

impl PublishCapabilities {
    #[cfg(target_os = "linux")]
    pub(super) fn exchange_supported<E>(
        &self,
        probe: impl FnOnce() -> Result<bool, E>,
    ) -> Result<bool, E> {
        if let Some(supported) = self.exchange_supported.get() {
            return Ok(supported);
        }
        let supported = probe()?;
        self.exchange_supported.set(Some(supported));
        #[cfg(test)]
        self.exchange_probes
            .set(self.exchange_probes.get().saturating_add(1));
        Ok(supported)
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn exchange_probe_count(&self) -> u64 {
        self.exchange_probes.get()
    }
}
