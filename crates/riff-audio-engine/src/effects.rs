use crate::{
    EqController, EqProcessor, MixController, MixProcessor, MonoController, MonoProcessor,
    PanController, PanProcessor, PitchController, PitchProcessor, ProcessorChain,
};

#[derive(Clone)]
pub struct Effects {
    pub eq: EqController,
    pub mono: MonoController,
    pub pan: PanController,
    pub pitch: PitchController,
    pub mix: MixController,
}

impl Effects {
    pub fn new(eq_bands: [f64; 10], mono: bool, pan: f64, pitch_cents: f64) -> Self {
        Self {
            eq: EqController::new(eq_bands),
            mono: MonoController::new(mono),
            pan: PanController::new(pan),
            pitch: PitchController::new(pitch_cents),
            mix: MixController::new(false),
        }
    }

    pub fn chain(&self) -> ProcessorChain {
        ProcessorChain::new()
            .with(Box::new(EqProcessor::new(self.eq.clone())))
            .with(Box::new(MonoProcessor::new(self.mono.clone())))
            .with(Box::new(PanProcessor::new(self.pan.clone())))
            .with(Box::new(PitchProcessor::new(self.pitch.clone())))
            .with(Box::new(MixProcessor::new(self.mix.clone())))
    }
}
