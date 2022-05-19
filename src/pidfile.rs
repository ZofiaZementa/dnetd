use anyhow::{Error, Result};
use memfd;
use memmap;
use std::io::Write;

pub struct PidFile {
    mapped_file: memmap::MmapMut,
    // never read, though still needed for the memory file to exist
    #[allow(dead_code)]
    file: memfd::Memfd,
}

impl PidFile {
    pub fn init() -> Result<Self> {
        let len = 12;
        let file = memfd::MemfdOptions::default()
            .allow_sealing(true)
            .create("/tmp/denetd_nextpid")?;
        file.as_file().set_len(len)?;
        let mapped_file = unsafe { memmap::MmapMut::map_mut(file.as_file())? };
        let mut seals = memfd::SealsHashSet::new();
        seals.insert(memfd::FileSeal::SealFutureWrite);
        seals.insert(memfd::FileSeal::SealGrow);
        seals.insert(memfd::FileSeal::SealShrink);
        file.add_seals(&seals)?;
        file.add_seal(memfd::FileSeal::SealSeal)?;
        Ok(PidFile { mapped_file, file })
    }

    pub fn set_pid(&mut self, pid: u32) -> Result<()> {
        self.mapped_file.as_mut().fill(0);
        self.mapped_file
            .as_mut()
            .write_all(format!("{}\n", pid).as_bytes())
            .map_err(Error::from)
    }
}
