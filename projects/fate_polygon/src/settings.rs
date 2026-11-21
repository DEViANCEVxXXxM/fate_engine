pub const MAX_BODIES: u32 = 384;
pub const MAX_RADIUS_HUNDREDTHS: u32 = 80;

#[derive(Clone, Copy)]
pub enum Preset {
    Mixed,
    Render,
    Physics,
    Agents,
    Navigation,
    Ecs,
}

impl Preset {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "mixed" => Some(Self::Mixed),
            "render" => Some(Self::Render),
            "physics" => Some(Self::Physics),
            "ai" => Some(Self::Agents),
            "navigation" => Some(Self::Navigation),
            "ecs" => Some(Self::Ecs),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mixed => "mixed",
            Self::Render => "render",
            Self::Physics => "physics",
            Self::Agents => "ai",
            Self::Navigation => "navigation",
            Self::Ecs => "ecs",
        }
    }
}

pub struct NumberSpec {
    pub name: &'static str,
    pub min: u32,
    pub max: u32,
    pub cpu: bool,
}

macro_rules! knobs {
    ($($field:ident: $label:literal, $page:ident, $min:literal..=$max:tt = $default:literal;)*) => {
        #[derive(Clone, Copy)]
        pub struct Settings {
            pub render: bool,
            pub physics: bool,
            pub ai: bool,
            pub ecs: bool,
            pub preview: bool,
            pub cull: bool,
            pub orbit: bool,
            $(pub $field: u32,)*
        }

        impl Default for Settings {
            fn default() -> Self {
                Self {
                    render: true, physics: true, ai: true, ecs: false, preview: true, cull: false, orbit: false,
                    $($field: $default,)*
                }
            }
        }

        impl Settings {
            pub const NUMBERS: &'static [NumberSpec] = &[$(
                NumberSpec {
                    name: stringify!($field), min: $min, max: $max,
                    cpu: matches!(fate_debug::Page::$page, fate_debug::Page::Simulation),
                },
            )*];

            pub fn number_value(self, id: &str) -> Option<u32> {
                match id {
                    $(stringify!($field) => Some(self.$field),)*
                    _ => None,
                }
            }

            pub fn apply_number(&mut self, id: &str, value: f64) -> bool {
                if !value.is_finite() {
                    return false;
                }
                match id {
                    $(stringify!($field) => self.$field = value.round().clamp($min as f64, $max as f64) as u32,)*
                    _ => return false,
                }
                true
            }

            pub fn number_controls(&self) -> impl Iterator<Item = fate_debug::Control> {
                [$({
                    let mut control = fate_debug::Control::number(
                        fate_debug::Page::$page, stringify!($field), $label,
                        self.$field as f64, $min as f64..=$max as f64,
                    );
                    if let fate_debug::ControlValue::Number { speed, .. } = &mut control.value {
                        *speed = 1.0;
                    }
                    control
                },)*].into_iter()
            }
        }
    };
}

knobs! {
    candidates: "Stress mesh candidates", Renderer, 0..=4096 = 128;
    detail: "Mesh tessellation", Renderer, 3..=48 = 10;
    batch: "Instances per material run", Renderer, 1..=256 = 256;
    frame_hz: "Redraw limit (Hz)", Renderer, 15..=120 = 60;
    seed: "Spawn seed (reset)", Simulation, 0..=1000000 = 71;
    tick_hz: "Fixed-step rate (Hz)", Simulation, 15..=240 = 120;
    bodies: "Physics / bodies", Simulation, 0..=MAX_BODIES = 72;
    broad_phase: "Physics / pair search (0 all pairs, 1 grid)", Simulation, 0..=1 = 0;
    passes: "Physics / pair passes per tick", Simulation, 1..=4 = 1;
    gravity: "Physics / downward acceleration", Simulation, 0..=40 = 10;
    radius: "Physics / radius (hundredths)", Simulation, 5..=MAX_RADIUS_HUNDREDTHS = 22;
    bounce: "Physics / restitution (%)", Simulation, 0..=100 = 78;
    agents: "AI / agents", Simulation, 0..=1024 = 128;
    samples: "AI / neighbors sampled", Simulation, 0..=96 = 12;
    think_every: "AI / ticks between decisions", Simulation, 1..=32 = 4;
    agent_speed: "AI / speed", Simulation, 0..=24 = 6;
    nav_mode: "AI / navigation (0 steering, 1 per-agent, 2 shared)", Simulation, 0..=2 = 0;
    nav_goal_ticks: "AI / ticks between navigation goals", Simulation, 16..=2048 = 240;
    entities: "ECS / live entities", Simulation, 0..=8192 = 2048;
    churn: "ECS / replacements per tick", Simulation, 0..=1024 = 128;
    snapshot_every: "ECS / snapshot cadence (0 off)", Simulation, 0..=1024 = 60;
}

impl Settings {
    pub const TOGGLES: &'static [&'static str] =
        &["render", "physics", "ai", "ecs", "preview", "cull", "orbit"];
    pub const ECS_NUMBERS: &'static [&'static str] = &["entities", "churn", "snapshot_every"];

    pub fn apply_toggle(&mut self, id: &str, value: bool) {
        match id {
            "render" => self.render = value,
            "physics" => self.physics = value,
            "ai" => self.ai = value,
            "ecs" => self.ecs = value,
            "preview" => self.preview = value,
            "cull" => self.cull = value,
            "orbit" => self.orbit = value,
            _ => {}
        }
    }

    pub fn preset(preset: Preset) -> Self {
        let base = Self::default();
        match preset {
            Preset::Mixed => base,
            Preset::Render => Self {
                physics: false,
                ai: false,
                candidates: 512,
                detail: 32,
                batch: 8,
                ..base
            },
            Preset::Physics => Self {
                render: false,
                ai: false,
                bodies: 256,
                passes: 2,
                ..base
            },
            Preset::Agents => Self {
                render: false,
                physics: false,
                agents: 768,
                samples: 48,
                think_every: 2,
                ..base
            },
            Preset::Navigation => Self {
                render: false,
                physics: false,
                agents: 256,
                think_every: 12,
                nav_mode: 2,
                ..base
            },
            Preset::Ecs => Self {
                render: false,
                physics: false,
                ai: false,
                ecs: true,
                ..base
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_clamp_and_presets_isolate_workloads() {
        let mut s = Settings::default();
        assert!(s.apply_number("bodies", 10_000.0));
        assert_eq!(s.bodies, 384);
        assert!(!s.apply_number("bodies", f64::NAN));
        assert_eq!(s.bodies, 384);
        assert!(s.apply_number("tick_hz", -20.0));
        assert_eq!(s.tick_hz, 15);
        assert!(!s.apply_number("unsupported", 1.0));
        for (preset, expected) in [
            (Preset::Mixed, (true, true, true)),
            (Preset::Render, (true, false, false)),
            (Preset::Physics, (false, true, false)),
            (Preset::Agents, (false, false, true)),
            (Preset::Navigation, (false, false, true)),
        ] {
            let s = Settings::preset(preset);
            assert_eq!((s.render, s.physics, s.ai), expected);
        }
    }
}
