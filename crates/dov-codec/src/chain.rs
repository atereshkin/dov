//! `Chain` — apply several codecs in series to model a tandem path.
//!
//! e.g. `Chain::new(vec![Cvsd, GsmFr, Cvsd])` models a Bluetooth-bridged GSM
//! call: `PC → BT(CVSD) → phone → GSM → phone → BT(CVSD) → PC`. Each member
//! keeps its own state, so chaining frame by frame preserves continuity.

use crate::{Codec, FRAME_LEN};

pub struct Chain {
    codecs: Vec<Box<dyn Codec>>,
    name: String,
}

impl Chain {
    pub fn new(codecs: Vec<Box<dyn Codec>>) -> Self {
        let name = codecs
            .iter()
            .map(|c| c.name())
            .collect::<Vec<_>>()
            .join("+");
        Self { codecs, name }
    }
}

impl Codec for Chain {
    fn name(&self) -> &str {
        &self.name
    }

    fn process_frame(&mut self, input: &[i16; FRAME_LEN]) -> [i16; FRAME_LEN] {
        let mut buf = *input;
        for codec in self.codecs.iter_mut() {
            buf = codec.process_frame(&buf);
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cvsd, GsmFr};

    #[test]
    fn chain_names_compose() {
        let chain = Chain::new(vec![
            Box::new(Cvsd::new()),
            Box::new(GsmFr::new()),
            Box::new(Cvsd::new()),
        ]);
        assert_eq!(chain.name(), "cvsd+gsm-fr+cvsd");
    }
}
