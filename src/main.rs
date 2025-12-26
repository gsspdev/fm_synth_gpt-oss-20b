// src/main.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::f32::consts::PI;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Simple ADSR envelope
#[derive(Clone, Copy)]
struct Envelope {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    phase: f32,   // 0: attack, 1: decay, 2: sustain, 3: release
    level: f32,
    active: bool,
}

impl Envelope {
    fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
            phase: 0.0,
            level: 0.0,
            active: false,
        }
    }

    fn note_on(&mut self) {
        self.phase = 0.0;
        self.level = 0.0;
        self.active = true;
    }

    fn note_off(&mut self) {
        self.phase = 3.0; // release
    }

    fn advance(&mut self, dt: f32) {
        if !self.active {
            return;
        }

        match self.phase {
            0.0 => { // attack
                self.level += dt / self.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.phase = 1.0;
                }
            }
            1.0 => { // decay
                self.level -= dt * (1.0 - self.sustain) / self.decay;
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    self.phase = 2.0;
                }
            }
            2.0 => { /* sustain – do nothing */ }
            3.0 => { // release
                self.level -= dt * self.sustain / self.release;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.active = false;
                }
            }
            _ => {}
        }
    }
}

/// An FM operator: oscillator + envelope
#[derive(Clone)]
struct Operator {
    freq: f32,
    phase: f32,
    amp: f32,
    envelope: Envelope,
}

impl Operator {
    fn new(freq: f32, amp: f32, envelope: Envelope) -> Self {
        Self {
            freq,
            phase: 0.0,
            amp,
            envelope,
        }
    }

    /// Compute one sample
    fn sample(&mut self, dt: f32, mod_in: f32) -> f32 {
        let mod_freq = self.freq + mod_in * self.freq;
        self.phase += 2.0 * PI * mod_freq * dt;
        self.phase = self.phase % (2.0 * PI);

        self.envelope.advance(dt);
        let env = self.envelope.level;

        self.amp * env * self.phase.sin()
    }
}

/// FM Synthesizer with one carrier and one modulator
struct FMSynth {
    carrier: Operator,
    modulator: Operator,
    sample_rate: f32,
}

impl FMSynth {
    fn new(carrier_freq: f32, mod_freq: f32, sample_rate: f32) -> Self {
        let env = Envelope::new(0.01, 0.1, 0.8, 0.2);
        Self {
            carrier: Operator::new(carrier_freq, 1.0, env.clone()),
            modulator: Operator::new(mod_freq, 1.0, env),
            sample_rate,
        }
    }

    fn note_on(&mut self) {
        self.carrier.envelope.note_on();
        self.modulator.envelope.note_on();
    }

    fn note_off(&mut self) {
        self.carrier.envelope.note_off();
        self.modulator.envelope.note_off();
    }

    fn render_block(&mut self, out: &mut [f32]) {
        let dt = 1.0 / self.sample_rate;
        for sample in out.iter_mut() {
            let mod_out = self.modulator.sample(dt, 0.0);
            *sample = self.carrier.sample(dt, mod_out);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Host / device / config
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No default output device");
    let config = device.default_output_config()?;

    // Shared synth state
    let synth = Arc::new(Mutex::new(FMSynth::new(
        440.0,                       // carrier
        220.0,                       // modulator
        config.sample_rate() as f32, // sample rate
    )));

    // Audio stream
    let synth_clone = synth.clone();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut synth = synth_clone.lock().unwrap();
                synth.render_block(data);
            },
            err_fn,
            None, // silence timeout (optional)
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config.into(),
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let mut synth = synth_clone.lock().unwrap();
                let mut buf = vec![0.0f32; data.len()];
                synth.render_block(&mut buf);
                for (sample, out) in buf.iter().zip(data.iter_mut()) {
                    *out = (*sample * i16::MAX as f32) as i16;
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config.into(),
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                let mut synth = synth_clone.lock().unwrap();
                let mut buf = vec![0.0f32; data.len()];
                synth.render_block(&mut buf);
                for (sample, out) in buf.iter().zip(data.iter_mut()) {
                    *out = ((*sample * i16::MAX as f32) as i16 as u16) + 32768;
                }
            },
            err_fn,
            None,
        )?,
        // Catch‑all for future sample formats
        _ => panic!("Unsupported sample format"),
    };

    stream.play()?;

    // Simple demo: play a note for 2 seconds
    {
        let mut synth = synth.lock().unwrap();
        synth.note_on();
    }
    thread::sleep(Duration::from_secs(2));

    {
        let mut synth = synth.lock().unwrap();
        synth.note_off();
    }
    thread::sleep(Duration::from_secs(1));

    Ok(())
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("Stream error: {}", err);
}
