use jack;

const MAX_AMPLITUDE: f32 = 0.5;

const ATTACK: usize = 2000;
const DECAY: usize = 2000;
const SUSTAIN: f32 = 0.6;
const RELEASE: usize = 5000;

fn main() {
    let (client, _status) = jack::Client::new("rust_client", jack::ClientOptions::NO_START_SERVER).unwrap();
    let midi_in_port = client.register_port("midi_in", jack::MidiIn::default()).unwrap();
    let mut audio_out_port = client.register_port("audio_out", jack::AudioOut::default()).unwrap();

    let mut frequencies: [f32; 128] = [0.0; 128];
    for (i, f) in frequencies.iter_mut().enumerate() {
        *f = 13.75 * 2.0_f32.powf((i as f32 - 9.0) / 12.0);
    }

    let mut synthesizer = Synthesizer::new(client.sample_rate(), frequencies);

    let process = jack::ClosureProcessHandler::new(
        move |_:&jack::Client, ps: &jack::ProcessScope| {
            for raw_midi in midi_in_port.iter(ps) {
                synthesizer.handle_midi(raw_midi);
            };

            for (frame, value) in audio_out_port.as_mut_slice(ps).iter_mut().enumerate() {
                *value = synthesizer.get_audio_data(frame as usize);
            }

            jack::Control::Continue
        }
    );

    let _active_client = client.activate_async((), process).unwrap();
    loop {}
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum EnvelopeStage {
    Attack(usize, f32, f32),
    Decay(usize, f32, f32),
    Sustain(f32),
    Release(usize, f32),
    Off,
}

#[derive(Copy, Clone, Debug)]
struct Note {
    frequency: f32,
    velocity: u8,
    time: usize,
    envelope_stage: EnvelopeStage,
    next_start_frame: Option<usize>,
}

impl Note {
    fn new(frequency: f32) -> Note {
        Note {
            frequency,
            velocity: 0,
            time: 0,
            envelope_stage: EnvelopeStage::Off,
            next_start_frame: None,
        }
    }

    fn increment_time(&mut self, frame: usize) {
        self.time += 1;

        if let Some(start_frame) = self.next_start_frame {
            if start_frame == frame {
                self.next_start_frame = None;
                self.envelope_stage = EnvelopeStage::Attack(0, self.amplitude(), self.fractional_velocity());
            }
        }

        match self.envelope_stage {
            EnvelopeStage::Attack(phase_timer, amplitude_start, amplitude_end) => {
                if phase_timer == ATTACK {
                    self.envelope_stage = EnvelopeStage::Decay(0, self.fractional_velocity(), self.fractional_velocity() * SUSTAIN);
                } else {
                    self.envelope_stage = EnvelopeStage::Attack(phase_timer + 1, amplitude_start, amplitude_end);
                }
            },
            EnvelopeStage::Decay(phase_timer, amplitude_start, amplitude_end) => {
                if phase_timer == DECAY {
                    self.envelope_stage = EnvelopeStage::Sustain(self.fractional_velocity() * SUSTAIN);
                } else {
                    self.envelope_stage = EnvelopeStage::Decay(phase_timer + 1, amplitude_start, amplitude_end);
                }
            },
            EnvelopeStage::Release(phase_timer, amplitude_start) => {
                if phase_timer == RELEASE {
                    self.envelope_stage = EnvelopeStage::Off;
                } else {
                    self.envelope_stage = EnvelopeStage::Release(phase_timer + 1, amplitude_start);
                }
            },
            _ => (),
        };
    }

    fn amplitude(&self) -> f32 {
        match self.envelope_stage {
            EnvelopeStage::Attack(phase_timer, amplitude_start, amplitude_end) => amplitude_start + (amplitude_end - amplitude_start) * phase_timer as f32 / ATTACK as f32,
            EnvelopeStage::Decay(phase_timer, amplitude_start, amplitude_end) => amplitude_start - (amplitude_start - amplitude_end) * phase_timer as f32 / DECAY as f32,
            EnvelopeStage::Sustain(amplitude) => amplitude,
            EnvelopeStage::Release(phase_timer, amplitude_start) => amplitude_start - amplitude_start * phase_timer as f32 / RELEASE as f32,
            EnvelopeStage::Off => 0.0,
        }
    }

    fn fractional_velocity(&self) -> f32 {
        self.velocity as f32 / 127.0
    }

    fn release(&mut self) {
        self.envelope_stage = EnvelopeStage::Release(0, self.amplitude());
    }
}

struct Synthesizer {
    time_step: f32,
    notes: [Note; 128],
}

impl Synthesizer {
    fn new(sample_rate: usize, frequencies: [f32; 128]) -> Synthesizer {
        let time_step = 1.0 / sample_rate as f32;
        let mut notes = [Note::new(0.0); 128];

        for i in 0..128 {
            notes[i].frequency = frequencies[i];
        }

        Synthesizer {
            time_step,
            notes,
        }
    }

    fn handle_midi(&mut self, raw_midi: jack::RawMidi) {
        let status = raw_midi.bytes[0];
        let pitch = raw_midi.bytes[1];
        let velocity = raw_midi.bytes[2];
        let start_time = raw_midi.time as usize;

        match status >> 4 {
            0b1000 => self.note_off(pitch),
            0b1001 => self.note_on(pitch, velocity, start_time),
            _ => (),
        };
    }

    fn note_on(&mut self, pitch: u8, velocity: u8, start_time: usize) {
        let pitch = pitch as usize;
        self.notes[pitch].velocity = velocity;
        self.notes[pitch].next_start_frame = Some(start_time);
    }

    fn note_off(&mut self, pitch: u8) {
        let pitch = pitch as usize;
        self.notes[pitch].release();
    }

    fn get_audio_data(&mut self, frame: usize) -> f32 {
        let mut value = 0.0;
        for note in self.notes.iter_mut() {
            if note.envelope_stage == EnvelopeStage::Off && note.next_start_frame.is_none() {
                continue;
            }

            let x: f32 = note.frequency * self.time_step * note.time as f32 * 2.0 * std::f32::consts::PI;
            let y = MAX_AMPLITUDE * note.amplitude() * x.sin();
            value += y;

            note.increment_time(frame);
        }
        value
    }
}
