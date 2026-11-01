//! Native output belongs to the worker, never to the guest. Missing devices are silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCommand {
    Play {
        channel: u8,
        asset: u32,
        looping: bool,
        volume: u16,
    },
    Stop(u8),
    Volume(u8, u16),
    Mute(bool),
}
#[cfg(feature = "native-audio")]
pub(crate) struct Output {
    stream: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    sinks: [Option<rodio::Sink>; 8],
    volumes: [u16; 8],
    muted: bool,
    paused: bool,
    pending: Vec<std::sync::Weak<()>>,
}
#[cfg(feature = "native-audio")]
impl Output {
    pub fn new(enabled: bool) -> Self {
        Self {
            stream: enabled
                .then(|| rodio::OutputStream::try_default().ok())
                .flatten(),
            sinks: std::array::from_fn(|_| None),
            volumes: [1000; 8],
            muted: false,
            paused: false,
            pending: Vec::new(),
        }
    }
    // Rodio's mixer has an internal queue. Bound live source admissions even if a
    // disconnected/stalled device stops consuming that queue. Weak guards require
    // no acknowledgements or waits on the real-time thread.
    fn admit(&mut self) -> Option<std::sync::Arc<()>> {
        self.pending.retain(|guard| guard.strong_count() != 0);
        if self.pending.len() >= 16 {
            return None;
        }
        let guard = std::sync::Arc::new(());
        self.pending.push(std::sync::Arc::downgrade(&guard));
        Some(guard)
    }
    pub fn apply(&mut self, commands: &[AudioCommand], assets: &crate::Assets) {
        use rodio::Source;
        for command in commands {
            match *command {
                AudioCommand::Play {
                    channel,
                    asset,
                    looping,
                    volume,
                } => {
                    let i = channel as usize;
                    self.sinks[i] = None;
                    self.volumes[i] = volume;
                    let Some(guard) = self.admit() else {
                        continue;
                    };
                    if let (Some((_, handle)), Some(sound)) = (&self.stream, assets.sound(asset)) {
                        if let Ok(sink) = rodio::Sink::try_new(handle) {
                            // Arc-backed sources reuse decoded samples for every sound and loop.
                            let source = Samples {
                                sound: sound.clone(),
                                position: 0,
                                _guard: guard,
                            };
                            if looping {
                                sink.append(source.repeat_infinite());
                            } else {
                                sink.append(source);
                            }
                            sink.set_volume(if self.muted {
                                0.
                            } else {
                                volume as f32 / 1000.
                            });
                            if self.paused {
                                sink.pause();
                            }
                            self.sinks[i] = Some(sink);
                        }
                    }
                }
                AudioCommand::Stop(channel) => self.sinks[channel as usize] = None,
                AudioCommand::Volume(channel, volume) => self.volumes[channel as usize] = volume,
                AudioCommand::Mute(muted) => self.muted = muted,
            }
        }
        for (i, sink) in self.sinks.iter().enumerate() {
            if let Some(sink) = sink {
                sink.set_volume(if self.muted {
                    0.
                } else {
                    self.volumes[i] as f32 / 1000.
                });
            }
        }
    }
    pub fn pause(&mut self, pause: bool) {
        self.paused = pause;
        for sink in self.sinks.iter().flatten() {
            if pause {
                sink.pause();
            } else {
                sink.play();
            }
        }
    }
    pub fn clear(&mut self) {
        self.sinks = std::array::from_fn(|_| None);
        self.muted = false;
    }
}
#[cfg(feature = "native-audio")]
#[derive(Clone)]
struct Samples {
    sound: crate::Sound,
    position: usize,
    _guard: std::sync::Arc<()>,
}
#[cfg(feature = "native-audio")]
impl Iterator for Samples {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.sound.samples.get(self.position).copied();
        if sample.is_some() {
            self.position += 1;
        }
        sample
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.sound.samples.len() - self.position;
        (remaining, Some(remaining))
    }
}
#[cfg(feature = "native-audio")]
impl rodio::Source for Samples {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.sound.samples.len() - self.position)
    }
    fn channels(&self) -> u16 {
        self.sound.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sound.rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs_f64(
            self.sound.samples.len() as f64 / self.sound.channels as f64 / self.sound.rate as f64,
        ))
    }
}
#[cfg(not(feature = "native-audio"))]
pub(crate) struct Output;
#[cfg(not(feature = "native-audio"))]
impl Output {
    pub fn new(_: bool) -> Self {
        Self
    }
    pub fn apply(&mut self, _: &[AudioCommand], _: &crate::Assets) {}
    pub fn pause(&self, _: bool) {}
    pub fn clear(&mut self) {}
}

#[cfg(all(test, feature = "native-audio"))]
mod tests {
    use super::*;
    #[test]
    fn native_channels_pause_stop_volume_mute_and_reset_without_a_device() {
        let mut output = Output::new(false);
        output.apply(
            &[AudioCommand::Play {
                channel: 0,
                asset: 1,
                looping: true,
                volume: 500,
            }],
            &crate::Assets::default(),
        );
        assert!(output.sinks.iter().all(Option::is_none));
        let (sink, _source) = rodio::Sink::new_idle();
        output.sinks[0] = Some(sink);
        output.pause(true);
        assert!(output.sinks[0].as_ref().unwrap().is_paused());
        output.pause(false);
        assert!(!output.sinks[0].as_ref().unwrap().is_paused());
        output.apply(&[AudioCommand::Volume(0, 250)], &crate::Assets::default());
        assert_eq!(output.sinks[0].as_ref().unwrap().volume(), 0.25);
        output.apply(&[AudioCommand::Mute(true)], &crate::Assets::default());
        assert_eq!(output.sinks[0].as_ref().unwrap().volume(), 0.);
        output.apply(&[AudioCommand::Mute(false)], &crate::Assets::default());
        assert_eq!(output.sinks[0].as_ref().unwrap().volume(), 0.25);
        output.apply(&[AudioCommand::Stop(0)], &crate::Assets::default());
        assert!(output.sinks[0].is_none());
        let (sink, _source) = rodio::Sink::new_idle();
        output.sinks[1] = Some(sink);
        output.clear();
        assert!(output.sinks.iter().all(Option::is_none));
    }
    #[test]
    fn stalled_native_output_cannot_accumulate_unbounded_sources_and_can_recover() {
        let mut output = Output::new(false);
        let mut stalled = Vec::new();
        for _ in 0..16 {
            let guard = output.admit().unwrap();
            let (sink, source) = rodio::Sink::new_idle();
            sink.append(Samples {
                sound: crate::Sound {
                    channels: 1,
                    rate: 8000,
                    samples: vec![0.5; 80].into(),
                },
                position: 0,
                _guard: guard,
            });
            // Keep the receiver alive without consuming it, like a stalled driver.
            stalled.push(source);
            drop(sink);
        }
        assert!(output.admit().is_none());
        output.clear();
        assert!(
            output.admit().is_none(),
            "reset must not bypass stalled-source admission"
        );
        drop(stalled);
        assert!(
            output.admit().is_some(),
            "consumed/dropped sources release admission"
        );
    }
}
