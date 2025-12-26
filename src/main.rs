
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use egui::Slider;
use eframe::egui;
use std::f32::consts::PI;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// ----------  Envelopes & Operators ----------
#[derive(Clone, Copy)]
struct Envelope {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    phase: f32,
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
    fn note_on(&mut self)   { self.phase = 0.0; self.level = 0.0; self.active = true; }
    fn note_off(&mut self)  { self.phase = 3.0; }          // release
    fn advance(&mut self, dt: f32) {
        if !self.active { return; }
        match self.phase {
            0.0 => {
                self.level += dt / self.attack;
                if self.level >= 1.0 { self.level = 1.0; self.phase = 1.0; }
            }
            1.0 => {
                self.level -= dt * (1.0 - self.sustain) / self.decay;
                if self.level <= self.sustain { self.level = self.sustain; self.phase = 2.0; }
            }
            2.0 => {}
            3.0 => {
                self.level -= dt * self.sustain / self.release;
                if self.level <= 0.0 { self.level = 0.0; self.active = false; }
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Operator {
    freq: f32,
    phase: f32,
    amp: f32,
    envelope: Envelope,
    ratio: f32,       // modulation ratio
    feedback: f32,    // self‑feedback [0..1]
    sync: bool,       // hard‑sync
    bit_depth: u8,    // 8–16 for bit‑crushing
}

impl Operator {
    fn new(freq: f32, amp: f32, env: Envelope,
           ratio: f32, feedback: f32, sync: bool, bit_depth: u8) -> Self {
        Self { freq, phase: 0.0, amp, envelope: env,
               ratio, feedback, sync, bit_depth }
    }

    fn crush(&self, sample: f32) -> f32 {
        let step = 2.0_f32.powi(-(self.bit_depth as i32));
        ((sample / step).round() * step).clamp(-1.0, 1.0)
    }

    fn hard_sync(&self, phase: f32) -> f32 {
        if self.sync { phase % (2.0 * PI) } else { phase }
    }

    fn sample(&mut self, dt: f32, mod_in: f32) -> f32 {
        let mod_freq = self.freq * self.ratio + mod_in * self.freq;
        let fb = self.feedback * self.phase;
        self.phase += 2.0 * PI * mod_freq * dt + fb;
        self.phase = self.hard_sync(self.phase);

        self.envelope.advance(dt);
        let env = self.envelope.level;

        let raw = self.amp * env * self.phase.sin();
        let clipped = raw.clamp(-0.9, 0.9);
        self.crush(clipped)
    }
}

/// ----------  Synth ----------
struct FMSynth {
    ops: [Operator; 4], // 0: carrier, 1: mod1, 2: mod2, 3: mod3
    sr: f32,
}

impl FMSynth {
    fn new(sr: f32) -> Self {
        let env = Envelope::new(0.01, 0.05, 0.6, 0.2);
        let ratios = [1.0, 1.618, 2.414, 3.732];
        let ops = [
            Operator::new(440.0, 1.0, env.clone(), ratios[0], 0.0, false, 16),
            Operator::new(220.0, 0.8, env.clone(), ratios[1], 0.05, true, 12),
            Operator::new(110.0, 0.6, env.clone(), ratios[2], 0.1, true, 10),
            Operator::new( 55.0, 0.4, env.clone(), ratios[3], 0.15, true, 8),
        ];
        Self { ops, sr }
    }

    fn note_on(&mut self)   { for o in &mut self.ops { o.envelope.note_on(); } }
    fn note_off(&mut self)  { for o in &mut self.ops { o.envelope.note_off(); } }

    fn render_block(&mut self, out: &mut [f32]) {
        let dt = 1.0 / self.sr;
        for s in out.iter_mut() {
            let m3 = self.ops[3].sample(dt, 0.0);
            let m2 = self.ops[2].sample(dt, m3);
            let m1 = self.ops[1].sample(dt, m2);
            *s = self.ops[0].sample(dt, m1);
        }
    }
}

/// ----------  UI App ----------
struct App {
    synth: Arc<Mutex<FMSynth>>,
    note_on: bool,
}

impl Default for App {
    fn default() -> Self { Self { synth: Arc::new(Mutex::new(FMSynth::new(44100.0))), note_on: false } }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("FM Synth Beast Control");

            // Operator panels
            let mut synth = self.synth.lock().unwrap();
            for (i, op) in synth.ops.iter_mut().enumerate() {
                ui.collapsing(format!("Operator {}", i), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Freq:"); ui.add(Slider::new(&mut op.freq, 20.0..=2000.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Amp:"); ui.add(Slider::new(&mut op.amp, 0.0..=2.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Ratio:"); ui.add(Slider::new(&mut op.ratio, 0.1..=5.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Feedback:"); ui.add(Slider::new(&mut op.feedback, 0.0..=0.5));
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut op.sync, "Sync");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Bit Depth:"); ui.add(Slider::new(&mut op.bit_depth, 8u8..=16));
                    });

                    // Envelope sliders
                    let e = &mut op.envelope;
                    ui.horizontal(|ui| {
                        ui.label("Attack"); ui.add(Slider::new(&mut e.attack, 0.001..=2.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Decay"); ui.add(Slider::new(&mut e.decay, 0.001..=2.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sustain"); ui.add(Slider::new(&mut e.sustain, 0.0..=1.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Release"); ui.add(Slider::new(&mut e.release, 0.001..=2.0));
                    });
                });
                ui.separator();
            }

            // Note button
            if ui.button(if self.note_on { "NOTE OFF" } else { "NOTE ON" }).clicked() {
                self.note_on = !self.note_on;
                if self.note_on { synth.note_on(); } else { synth.note_off(); }
            }
        });
    }
}

/// ----------  Main ----------
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Audio thread
    let host = cpal::default_host();
    let device = host.default_output_device().expect("No default device");
    let config = device.default_output_config()?;

    let synth = Arc::new(Mutex::new(FMSynth::new(
        config.sample_rate() as f32,
    )));

    let synth_a = synth.clone();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut synth = synth_a.lock().unwrap();
                synth.render_block(data);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config.into(),
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let mut synth = synth_a.lock().unwrap();
                let mut buf = vec![0.0f32; data.len()];
                synth.render_block(&mut buf);
                for (s, out) in buf.iter().zip(data.iter_mut()) {
                    *out = (*s * i16::MAX as f32) as i16;
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config.into(),
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                let mut synth = synth_a.lock().unwrap();
                let mut buf = vec![0.0f32; data.len()];
                synth.render_block(&mut buf);
                for (s, out) in buf.iter().zip(data.iter_mut()) {
                    *out = ((*s * i16::MAX as f32) as i16 as u16) + 32768;
                }
            },
            err_fn,
            None,
        )?,
        _ => panic!("Unsupported sample format"),
    };
    stream.play()?;

    // UI thread
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "FM Synth Beast",
        native_options,
        Box::new(|_cc| Box::new(App { synth, note_on: false })),
    )?;

    Ok(())
}

/// ----------  Error callback ----------
fn err_fn(err: cpal::StreamError) {
    eprintln!("Stream error: {}", err);
}
