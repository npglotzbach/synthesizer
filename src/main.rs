use jack;

const MAX_AMPLITUDE: f32 = 0.2;

const ATTACK: usize = 2000;
const DECAY: usize = 5000;
const SUSTAIN: f32 = 0.6;
const RELEASE: usize = 10000;

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

            synthesizer.notes_gc();

            jack::Control::Continue
        }
    );

    let _active_client = client.activate_async((), process).unwrap();
    loop {}
}

#[derive(Copy, Clone, PartialEq)]
enum EnvelopePhase {
    Stage(usize),
    Attack(usize),
    Decay(usize),
    Sustain,
    Release(usize, f32),
    Off,
}

#[derive(Copy, Clone)]
struct Note {
    pitch: u8,
    velocity: u8,
    time: usize,
    env_phase: EnvelopePhase,
}

impl Note {
    fn new(pitch: u8, velocity: u8, start_time: usize) -> Note {
        Note {
            pitch,
            velocity,
            time: 0,
            env_phase: EnvelopePhase::Stage(start_time),
        }
    }

    fn increment_time(&mut self, time: usize) {
        self.time += 1;

        if let EnvelopePhase::Stage(start_time) = self.env_phase {
            if start_time == time {
                self.env_phase = EnvelopePhase::Attack(0);
            }
        } else if let EnvelopePhase::Attack(phase_timer) = self.env_phase {
            if phase_timer == ATTACK {
                self.env_phase = EnvelopePhase::Decay(0);
            } else {
                self.env_phase = EnvelopePhase::Attack(phase_timer + 1);
            }
        } else if let EnvelopePhase::Decay(phase_timer) = self.env_phase {
            if phase_timer == DECAY {
                self.env_phase = EnvelopePhase::Sustain;
            } else {
                self.env_phase = EnvelopePhase::Decay(phase_timer + 1);
            }
        } else if let EnvelopePhase::Release(phase_timer, released_amplitude) = self.env_phase {
            if phase_timer >= RELEASE {
                self.env_phase = EnvelopePhase::Off;
            } else {
                self.env_phase = EnvelopePhase::Release(phase_timer + 1, released_amplitude);
            }
        }
    }

    fn amplitude(&self) -> f32 {
        match self.env_phase {
            EnvelopePhase::Stage(_) => 0.0,
            EnvelopePhase::Attack(phase_timer) => phase_timer as f32 / ATTACK as f32,
            EnvelopePhase::Decay(phase_timer) => 1.0 - ((1.0 - SUSTAIN) * phase_timer as f32 / DECAY as f32),
            EnvelopePhase::Sustain => SUSTAIN,
            EnvelopePhase::Release(phase_timer, released_amplitude) => released_amplitude - (released_amplitude * phase_timer as f32 / RELEASE as f32),
            EnvelopePhase::Off => 0.0,
        }
    }

    fn release(&mut self) {
        self.env_phase = EnvelopePhase::Release(0, self.amplitude());
    }
}

struct Synthesizer {
    time_step: f32,
    notes: Vec<Note>,
    frequencies: [f32; 128],
}

impl Synthesizer {
    fn new(sample_rate: usize, frequencies: [f32; 128]) -> Synthesizer {
        let time_step = 1.0 / sample_rate as f32;
        let notes = Vec::new();

        Synthesizer {
            time_step,
            notes,
            frequencies,
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
            _ => return,
        };
    }

    fn note_on(&mut self, pitch: u8, velocity: u8, start_time: usize) {
        self.notes.push(Note::new(pitch, velocity, start_time));
    }

    fn note_off(&mut self, pitch: u8) {
        for note in self.notes.iter_mut() {
            if note.pitch == pitch {
                note.release();
            }
        }
    }

    fn get_audio_data(&mut self, frame: usize) -> f32 {
        let mut value = 0.0;
        for note in self.notes.iter_mut() {
            let x: f32 = self.frequencies[note.pitch as usize] * self.time_step * note.time as f32 * 2.0 * std::f32::consts::PI;
            let y = MAX_AMPLITUDE * note.amplitude() * note.velocity as f32 / 127.0 * x.sin();
            value += y;

            note.increment_time(frame);
        }
        value
    }

    fn notes_gc(&mut self) {
        for i in (0..self.notes.len()).rev() {
            if self.notes[i].env_phase == EnvelopePhase::Off {
                self.notes.swap_remove(i);
            }
        }
    }
}
