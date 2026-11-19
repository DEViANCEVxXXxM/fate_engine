use fate_sim::math::{Fixed, FixedVec2, FixedVec3, segment_sphere_contact};
use fate_sim::{StateHasher, StateWriter};

pub const HZ: u32 = 60;
pub const LIMIT: u32 = HZ * 60;
pub const HEALTH: u8 = 6;
pub const ARENA: Fixed = Fixed::from_int(6);
pub const FULL_AXIS: i8 = i8::MAX;
const BOLT_BOUND: Fixed = Fixed::from_int(8);
const REACH: Fixed = Fixed::from_int(3);
const MAX_BOLTS: usize = 64;
const MAX_SHOTS: usize = 8;
pub const SNAPSHOT_MAX: usize = 126 + (MAX_BOLTS + MAX_SHOTS) * 32;
const WAVE_CYCLE: u32 = 64;
const WAVE_LOCK: u32 = 8;
const FIRST_VOLLEY: u32 = 24;
const EXTRA_VOLLEY: u32 = 34;
const RANGED_OPEN: u32 = 40;
const SHIFT_TICKS: u32 = 30;
const FIRST_STRIKE: u32 = 36;
const LAST_STRIKE: u32 = 45;
const EXTRA_STRIKE: u32 = 54;
const LUNGE_LOCK: u32 = 24;
const LUNGE_START: u32 = 36;
const LUNGE_TRAVEL: u32 = 8;
const LUNGE_END: u32 = LUNGE_START + LUNGE_TRAVEL - 1;
const LUNGE_RANGE: Fixed = Fixed::from_int(5);
const LUNGE_RADIUS: Fixed = Fixed::from_ratio(3, 4);
const GUARD_TICKS: u8 = 4;
const DASH_TICKS: u8 = 6;
const DASH_COOLDOWN: u8 = 28;
const OPENING: u8 = 12;
const COOLDOWN: u8 = 24;
const MAGIC: [u8; 4] = *b"DLAB";
const VERSION: u16 = 5;

#[derive(Clone, Copy, Debug, Default)]
pub struct Command {
    /// Quantized movement in `-FULL_AXIS..=FULL_AXIS`.
    pub x: i8,
    pub z: i8,
    pub aim_x: i8,
    pub aim_z: i8,
    pub guard: bool,
    pub attack: bool,
    pub boost: bool,
    pub dash: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Waves,
    Shift,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepCue {
    None,
    Dash,
    Parry,
    Shot,
    Blocked,
    Hit,
    Volley,
    Lunge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Active,
    Won,
    Lost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bolt {
    pub position: FixedVec2,
    velocity: FixedVec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LungeContact {
    Pending,
    Avoided,
    Hit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lunge {
    from: FixedVec2,
    to: FixedVec2,
    contact: LungeContact,
}

impl Lunge {
    fn lock(from: FixedVec2, player: FixedVec2) -> Self {
        let toward = player - from;
        let to = if toward.length_squared() > LUNGE_RANGE * LUNGE_RANGE {
            clamp_to_arena(from + unit_or_forward(toward) * LUNGE_RANGE)
        } else {
            player
        };
        Self {
            from,
            to,
            contact: LungeContact::Pending,
        }
    }

    fn position_at(self, beat: u32) -> FixedVec2 {
        if beat < LUNGE_START {
            self.from
        } else if beat >= LUNGE_END {
            self.to
        } else {
            let progress =
                Fixed::from_ratio(i64::from(beat - LUNGE_START + 1), i64::from(LUNGE_TRAVEL));
            self.from + (self.to - self.from) * progress
        }
    }
}

struct WavePattern {
    rays: [FixedVec2; 8],
    count: usize,
    speed: Fixed,
}

impl WavePattern {
    fn directions(&self) -> &[FixedVec2] {
        &self.rays[..self.count]
    }
}

#[derive(Debug)]
pub struct Duel {
    pub tick: u32,
    age: u32,
    pub phase: Phase,
    pub outcome: Outcome,
    pub health: u8,
    pub target_health: u8,
    pub charge: u8,
    pub pressure: u8,
    pub position: FixedVec2,
    pub opponent_position: FixedVec2,
    lunge: Option<Lunge>,
    opening: u8,
    cooldown: u8,
    guard_held: bool,
    guard_ticks: u8,
    combo_clean: bool,
    close_overloaded: bool,
    dash_held: bool,
    dash_ticks: u8,
    dash_cooldown: u8,
    dash_direction: FixedVec2,
    locked_wave_aim: FixedVec2,
    wave_overloaded: bool,
    pub bolts: Vec<Bolt>,
    pub shots: Vec<Bolt>,
}

fn vec(x: i64, z: i64) -> FixedVec2 {
    FixedVec2::new(Fixed::from_int(x), Fixed::from_int(z))
}

fn in_plane(p: FixedVec2) -> FixedVec3 {
    FixedVec3::new(p.x, Fixed::ZERO, p.y)
}

fn in_bounds(p: FixedVec2, extent: Fixed) -> bool {
    (-extent..=extent).contains(&p.x) && (-extent..=extent).contains(&p.y)
}

fn clamp_to_arena(p: FixedVec2) -> FixedVec2 {
    FixedVec2::new(p.x.clamp(-ARENA, ARENA), p.y.clamp(-ARENA, ARENA))
}

fn unit_or_forward(direction: FixedVec2) -> FixedVec2 {
    let direction = direction.normalize();
    if direction == FixedVec2::ZERO {
        return vec(0, 1);
    }
    // The fixed square root rounds down, so an axis can be a few bits over 1.
    FixedVec2::new(
        direction.x.clamp(-Fixed::ONE, Fixed::ONE),
        direction.y.clamp(-Fixed::ONE, Fixed::ONE),
    )
}

fn outcome_at(tick: u32, health: u8, target_health: u8) -> Outcome {
    match (health, target_health) {
        (0, _) => Outcome::Lost,
        (_, 0) => Outcome::Won,
        _ if tick == LIMIT => Outcome::Lost,
        _ => Outcome::Active,
    }
}

impl Duel {
    pub fn new() -> Self {
        Self {
            tick: 0,
            age: 0,
            phase: Phase::Waves,
            outcome: Outcome::Active,
            health: HEALTH,
            target_health: 3,
            charge: 0,
            pressure: 0,
            position: vec(0, 2),
            opponent_position: FixedVec2::ZERO,
            lunge: None,
            opening: 0,
            cooldown: 0,
            guard_held: false,
            guard_ticks: 0,
            combo_clean: false,
            close_overloaded: false,
            dash_held: false,
            dash_ticks: 0,
            dash_cooldown: 0,
            dash_direction: FixedVec2::ZERO,
            locked_wave_aim: FixedVec2::ZERO,
            wave_overloaded: false,
            bolts: Vec::with_capacity(MAX_BOLTS),
            shots: Vec::with_capacity(MAX_SHOTS),
        }
    }

    #[cfg(feature = "windowed")]
    pub fn telegraph(&self) -> &'static str {
        match self.phase {
            Phase::Waves => match self.age % WAVE_CYCLE {
                WAVE_LOCK..FIRST_VOLLEY if self.is_fan(false) => "FAN LOCKED - SIDESTEP",
                WAVE_LOCK..FIRST_VOLLEY => "RADIAL WINDUP - FIND A GAP",
                beat if self.wave_overloaded
                    && (FIRST_VOLLEY + 1..EXTRA_VOLLEY).contains(&beat) =>
                {
                    "EXTRA VOLLEY - KEEP MOVING"
                }
                FIRST_VOLLEY..RANGED_OPEN => "EVADE VOLLEY",
                RANGED_OPEN.. => "TARGET OPEN",
                _ => "WATCH",
            },
            Phase::Shift => "CLOSING IN - GET IN RANGE",
            Phase::Close if self.opening != 0 => "COUNTER NOW",
            Phase::Close if self.lunge.is_some() => match self.age % HZ {
                LUNGE_LOCK..LUNGE_START => "LUNGE LOCKED - STEP ASIDE",
                LUNGE_START..=LUNGE_END => "LUNGE",
                _ => "WATCH",
            },
            Phase::Close => match self.age % HZ {
                32..FIRST_STRIKE => "PARRY FIRST",
                41..LAST_STRIKE => "PARRY SECOND",
                50..EXTRA_STRIKE if self.close_overloaded => "PARRY THIRD",
                LAST_STRIKE..50 if self.close_overloaded => "COMBO WINDUP",
                24..32 | FIRST_STRIKE..41 => "COMBO WINDUP",
                _ => "WATCH",
            },
        }
    }

    #[cfg(feature = "windowed")]
    pub fn defense(&self) -> (bool, bool) {
        (self.dash_ticks != 0, self.guard_ticks != 0)
    }

    #[cfg(feature = "windowed")]
    pub fn wave_warning(&self) -> Option<([FixedVec2; 8], usize, u32)> {
        if self.phase != Phase::Waves || self.outcome != Outcome::Active {
            return None;
        }
        let beat = self.age % WAVE_CYCLE;
        let (extra, release) = if (WAVE_LOCK..FIRST_VOLLEY).contains(&beat) {
            (false, FIRST_VOLLEY)
        } else if self.wave_overloaded && (FIRST_VOLLEY + 1..EXTRA_VOLLEY).contains(&beat) {
            (true, EXTRA_VOLLEY)
        } else {
            return None;
        };
        let pattern = self.wave_pattern(extra);
        Some((pattern.rays, pattern.count, release - beat))
    }

    #[cfg(feature = "windowed")]
    pub fn close_warning(&self) -> Option<(Fixed, u32)> {
        if self.phase != Phase::Close || self.outcome != Outcome::Active || self.lunge.is_some() {
            return None;
        }
        let beat = self.age % HZ;
        let strike = match beat {
            24..FIRST_STRIKE => FIRST_STRIKE,
            FIRST_STRIKE..LAST_STRIKE => LAST_STRIKE,
            LAST_STRIKE..EXTRA_STRIKE if self.close_overloaded => EXTRA_STRIKE,
            _ => return None,
        };
        Some((REACH, strike - beat))
    }

    #[cfg(feature = "windowed")]
    pub fn lunge_warning(&self) -> Option<(FixedVec2, FixedVec2, Fixed, u32)> {
        let beat = self.age % HZ;
        let plan = self.lunge?;
        if self.outcome != Outcome::Active || beat > LUNGE_END {
            return None;
        }
        Some((
            plan.from,
            plan.to,
            LUNGE_RADIUS,
            LUNGE_START.saturating_sub(beat),
        ))
    }

    pub fn step(&mut self, command: Command) -> StepCue {
        if self.outcome != Outcome::Active {
            return StepCue::None;
        }
        self.tick += 1;
        self.age += 1;
        self.opening = self.opening.saturating_sub(1);
        self.cooldown = self.cooldown.saturating_sub(1);
        self.guard_ticks = self.guard_ticks.saturating_sub(1);
        self.dash_ticks = self.dash_ticks.saturating_sub(1);
        self.dash_cooldown = self.dash_cooldown.saturating_sub(1);
        let fresh_guard = !std::mem::replace(&mut self.guard_held, command.guard) && command.guard;
        let fresh_dash = !std::mem::replace(&mut self.dash_held, command.dash) && command.dash;
        if fresh_guard {
            self.guard_ticks = GUARD_TICKS;
        }

        let mut cue = StepCue::None;
        let previous = self.position;
        let movement = vec(
            i64::from(command.x.max(-FULL_AXIS)),
            i64::from(command.z.max(-FULL_AXIS)),
        ) / Fixed::from_int(i64::from(FULL_AXIS));
        let movement = if movement.length_squared() > Fixed::ONE {
            movement.normalize()
        } else {
            movement
        };
        if fresh_dash && self.dash_cooldown == 0 {
            let requested = if movement == FixedVec2::ZERO {
                previous
            } else {
                movement
            };
            self.dash_direction = unit_or_forward(requested);
            self.dash_ticks = DASH_TICKS;
            self.dash_cooldown = DASH_COOLDOWN;
            cue = StepCue::Dash;
        } else if self.dash_ticks == 0 {
            self.dash_direction = FixedVec2::ZERO;
        }
        let dashing = self.dash_ticks != 0;
        let moved = previous
            + if dashing {
                self.dash_direction * Fixed::from_ratio(3, 8)
            } else {
                movement * Fixed::from_ratio(1, 8)
            };
        self.position = clamp_to_arena(moved);

        if command.boost && self.charge >= 2 && self.health < HEALTH && self.pressure == 0 {
            self.charge -= 2;
            self.health = (self.health + 2).min(HEALTH);
            self.pressure = 1;
        }

        match self.phase {
            Phase::Close => {
                let close_cue = self.step_close(command, dashing, previous);
                if cue == StepCue::None {
                    cue = close_cue;
                }
            }
            Phase::Shift => {
                if self.age == SHIFT_TICKS {
                    self.phase = Phase::Close;
                    self.age = 0;
                }
            }
            Phase::Waves => {
                let wave_cue = self.step_waves(command, dashing, previous);
                if cue == StepCue::None {
                    cue = wave_cue;
                }
            }
        }

        self.outcome = outcome_at(self.tick, self.health, self.target_health);
        cue
    }

    fn step_close(&mut self, command: Command, dashing: bool, previous: FixedVec2) -> StepCue {
        let beat = self.age % HZ;
        if beat == 0 {
            self.lunge = None;
        } else if beat == 1 {
            self.close_overloaded = self.pressure != 0;
        }
        if beat == LUNGE_LOCK && !self.within_reach() {
            self.lunge = Some(Lunge::lock(self.opponent_position, self.position));
        }

        let cue = if let Some(plan) = self.lunge {
            self.step_lunge(plan, beat, dashing, previous)
        } else {
            self.step_combo(beat, dashing)
        };
        if self.health != 0 && self.within_reach() && self.opening > 0 && command.attack && !dashing
        {
            self.opening = 0;
            self.target_health -= 1;
        }
        cue
    }

    fn within_reach(&self) -> bool {
        (self.position - self.opponent_position).length_squared() <= REACH * REACH
    }

    fn parry(&mut self) {
        self.guard_ticks = 0;
        self.charge = (self.charge + 1).min(2);
    }

    fn step_combo(&mut self, beat: u32, dashing: bool) -> StepCue {
        let final_beat = if self.close_overloaded {
            EXTRA_STRIKE
        } else {
            LAST_STRIKE
        };
        if !matches!(beat, FIRST_STRIKE | LAST_STRIKE) && beat != final_beat {
            return StepCue::None;
        }
        let threatened = self.within_reach() && !dashing;
        let parried = threatened && self.guard_ticks > 0;
        if parried {
            self.parry();
        } else if threatened {
            self.health -= 1;
        }
        if beat == FIRST_STRIKE {
            self.combo_clean = parried;
        } else if beat == final_beat {
            if self.combo_clean && parried {
                self.opening = OPENING;
            }
            self.combo_clean = false;
        } else {
            self.combo_clean &= parried;
        }
        if parried {
            StepCue::Parry
        } else {
            StepCue::None
        }
    }

    fn step_lunge(
        &mut self,
        mut plan: Lunge,
        beat: u32,
        dashing: bool,
        previous: FixedVec2,
    ) -> StepCue {
        if !(LUNGE_START..=LUNGE_END).contains(&beat) {
            return StepCue::None;
        }
        let before = self.opponent_position;
        self.opponent_position = plan.position_at(beat);
        let mut cue = if beat == LUNGE_START {
            StepCue::Lunge
        } else {
            StepCue::None
        };
        if plan.contact == LungeContact::Pending
            && segment_sphere_contact(
                in_plane(previous - before),
                in_plane(self.position - self.opponent_position),
                FixedVec3::ZERO,
                LUNGE_RADIUS,
            )
            .is_some()
        {
            plan.contact = if dashing {
                LungeContact::Avoided
            } else if self.guard_ticks > 0 {
                self.parry();
                cue = StepCue::Parry;
                LungeContact::Avoided
            } else {
                self.health -= 1;
                LungeContact::Hit
            };
        }
        if beat == LUNGE_END && plan.contact != LungeContact::Hit {
            self.opening = OPENING;
        }
        self.lunge = Some(plan);
        cue
    }

    fn step_waves(&mut self, command: Command, dashing: bool, previous: FixedVec2) -> StepCue {
        let beat = self.age % WAVE_CYCLE;
        if beat == 0 {
            self.locked_wave_aim = FixedVec2::ZERO;
            self.wave_overloaded = false;
        } else if beat == WAVE_LOCK {
            self.locked_wave_aim = unit_or_forward(self.position);
            self.wave_overloaded = self.pressure != 0;
        }
        let released = beat == FIRST_VOLLEY || (beat == EXTRA_VOLLEY && self.wave_overloaded);
        if released {
            self.emit_wave(beat == EXTRA_VOLLEY);
        }
        let next = self.position;
        let health = &mut self.health;
        self.bolts.retain_mut(|bolt| {
            let start = bolt.position;
            bolt.position += bolt.velocity;
            if !dashing
                && segment_sphere_contact(
                    in_plane(start - previous),
                    in_plane(bolt.position - next),
                    FixedVec3::ZERO,
                    Fixed::from_ratio(1, 3),
                )
                .is_some()
            {
                *health = health.saturating_sub(1);
                return false;
            }
            in_bounds(bolt.position, BOLT_BOUND)
        });
        if self.health == 0 {
            return StepCue::None;
        }

        let mut cue = if released {
            StepCue::Volley
        } else {
            StepCue::None
        };
        let aim = vec(
            i64::from(command.aim_x.max(-FULL_AXIS)),
            i64::from(command.aim_z.max(-FULL_AXIS)),
        );
        if command.attack
            && !dashing
            && aim != FixedVec2::ZERO
            && self.cooldown == 0
            && self.shots.len() < MAX_SHOTS
        {
            self.cooldown = COOLDOWN;
            self.shots.push(Bolt {
                position: self.position,
                velocity: aim.normalize() * Fixed::from_ratio(3, 4),
            });
            if !released {
                cue = StepCue::Shot;
            }
        }
        let mut impacts = 0;
        self.shots.retain_mut(|shot| {
            let start = shot.position;
            shot.position += shot.velocity;
            if segment_sphere_contact(
                in_plane(start),
                in_plane(shot.position),
                FixedVec3::ZERO,
                Fixed::from_ratio(13, 20),
            )
            .is_some()
            {
                impacts += 1;
                return false;
            }
            in_bounds(shot.position, BOLT_BOUND)
        });
        if impacts > 0 && beat >= RANGED_OPEN {
            self.target_health = self.target_health.saturating_sub(impacts);
            self.charge = (self.charge + impacts).min(2);
            cue = StepCue::Hit;
            if self.target_health == 0 {
                self.phase = Phase::Shift;
                self.age = 0;
                self.target_health = 2;
                self.cooldown = 0;
                self.bolts.clear();
                self.shots.clear();
                self.locked_wave_aim = FixedVec2::ZERO;
                self.wave_overloaded = false;
            }
        } else if impacts > 0 {
            cue = StepCue::Blocked;
        }
        cue
    }

    fn is_fan(&self, extra: bool) -> bool {
        !(self.age / WAVE_CYCLE).is_multiple_of(2) ^ extra
    }

    fn wave_pattern(&self, extra: bool) -> WavePattern {
        const DIRECTIONS: [(i64, i64); 8] = [
            (1, 0),
            (1, 1),
            (0, 1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
        ];
        if self.is_fan(extra) {
            let forward = self.locked_wave_aim;
            let lateral = FixedVec2::new(forward.y, -forward.x);
            let mut rays = [FixedVec2::ZERO; 8];
            for (ray, sideways) in rays.iter_mut().zip(-2..=2) {
                *ray = (forward + lateral * Fixed::from_ratio(sideways, 4)).normalize();
            }
            WavePattern {
                rays,
                count: 5,
                speed: Fixed::from_ratio(1, 5),
            }
        } else {
            WavePattern {
                rays: DIRECTIONS.map(|(x, z)| vec(x, z).normalize()),
                count: DIRECTIONS.len(),
                speed: Fixed::from_ratio(1, 6),
            }
        }
    }

    fn emit_wave(&mut self, extra: bool) {
        let pattern = self.wave_pattern(extra);
        for &direction in pattern
            .directions()
            .iter()
            .take(MAX_BOLTS - self.bolts.len())
        {
            self.bolts.push(Bolt {
                position: direction * Fixed::from_ratio(1, 2),
                velocity: direction * pattern.speed,
            });
        }
    }

    fn write(&self, out: &mut impl StateWriter) {
        out.write_bytes(&MAGIC);
        out.write_u16(VERSION);
        out.write_u32(self.tick);
        out.write_u32(self.age);
        out.write_u8(match self.phase {
            Phase::Waves => 0,
            Phase::Shift => 1,
            Phase::Close => 2,
        });
        out.write_u8(match self.outcome {
            Outcome::Active => 0,
            Outcome::Won => 1,
            Outcome::Lost => 2,
        });
        out.write_bytes(&[
            self.health,
            self.target_health,
            self.charge,
            self.pressure,
            self.opening,
            self.cooldown,
            u8::from(self.guard_held)
                | (u8::from(self.combo_clean) << 1)
                | (u8::from(self.dash_held) << 2)
                | (u8::from(self.close_overloaded) << 3)
                | (u8::from(self.wave_overloaded) << 4),
            self.guard_ticks,
            self.dash_ticks,
            self.dash_cooldown,
        ]);
        write_vector(out, self.position);
        write_vector(out, self.dash_direction);
        write_vector(out, self.locked_wave_aim);
        write_vector(out, self.opponent_position);
        if let Some(plan) = self.lunge {
            out.write_u8(1);
            write_vector(out, plan.from);
            write_vector(out, plan.to);
            out.write_u8(match plan.contact {
                LungeContact::Pending => 0,
                LungeContact::Avoided => 1,
                LungeContact::Hit => 2,
            });
        } else {
            out.write_u8(0);
        }
        for projectiles in [&self.bolts, &self.shots] {
            out.write_u8(projectiles.len() as u8);
            for bolt in projectiles {
                write_vector(out, bolt.position);
                write_vector(out, bolt.velocity);
            }
        }
    }

    pub fn snapshot(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.write(&mut bytes);
        bytes
    }

    pub fn hash(&self) -> u64 {
        let mut hash = StateHasher::new();
        self.write(&mut hash);
        hash.finish()
    }

    pub fn restore(mut bytes: &[u8]) -> Result<Self, &'static str> {
        if take::<4>(&mut bytes)? != MAGIC || u16::from_le_bytes(take(&mut bytes)?) != VERSION {
            return Err("unsupported snapshot");
        }
        let tick = u32::from_le_bytes(take(&mut bytes)?);
        let age = u32::from_le_bytes(take(&mut bytes)?);
        let [
            phase,
            outcome,
            health,
            target_health,
            charge,
            pressure,
            opening,
            cooldown,
            flags,
            guard_ticks,
            dash_ticks,
            dash_cooldown,
        ] = take(&mut bytes)?;
        let phase = match phase {
            0 => Phase::Waves,
            1 => Phase::Shift,
            2 => Phase::Close,
            _ => return Err("invalid phase"),
        };
        let outcome = match outcome {
            0 => Outcome::Active,
            1 => Outcome::Won,
            2 => Outcome::Lost,
            _ => return Err("invalid outcome"),
        };
        let position = read_vector(&mut bytes)?;
        let dash_direction = read_vector(&mut bytes)?;
        let locked_wave_aim = read_vector(&mut bytes)?;
        let opponent_position = read_vector(&mut bytes)?;
        let [has_lunge] = take(&mut bytes)?;
        let lunge = match has_lunge {
            0 => None,
            1 => {
                let from = read_vector(&mut bytes)?;
                let to = read_vector(&mut bytes)?;
                let contact = match take(&mut bytes)? {
                    [0] => LungeContact::Pending,
                    [1] => LungeContact::Avoided,
                    [2] => LungeContact::Hit,
                    _ => return Err("invalid lunge contact"),
                };
                Some(Lunge { from, to, contact })
            }
            _ => return Err("invalid lunge presence"),
        };
        let combo_clean = flags & 0b0010 != 0;
        let close_overloaded = flags & 0b1000 != 0;
        let wave_overloaded = flags & 0b1_0000 != 0;
        let wave_locked = phase == Phase::Waves && age % WAVE_CYCLE >= WAVE_LOCK;
        let final_beat = if close_overloaded {
            EXTRA_STRIKE
        } else {
            LAST_STRIKE
        };
        let beat = age % HZ;
        let valid_lunge = lunge.is_none_or(|plan| {
            phase == Phase::Close
                && beat >= LUNGE_LOCK
                && in_bounds(plan.from, ARENA)
                && in_bounds(plan.to, ARENA)
                && (plan.to - plan.from).length_squared()
                    <= LUNGE_RANGE * LUNGE_RANGE + Fixed::from_ratio(1, 1000)
                && plan.from != plan.to
                && !combo_clean
                && (beat >= LUNGE_START || plan.contact == LungeContact::Pending)
                && (opening == 0 || (beat >= LUNGE_END && plan.contact != LungeContact::Hit))
                && opponent_position == plan.position_at(beat)
        });
        let bolts = read_projectiles(&mut bytes, MAX_BOLTS)?;
        let shots = read_projectiles(&mut bytes, MAX_SHOTS)?;
        let has_projectiles = !bolts.is_empty() || !shots.is_empty();
        if tick > LIMIT
            || age > tick
            || outcome != outcome_at(tick, health, target_health)
            || health > HEALTH
            || charge > 2
            || pressure > 1
            || opening > OPENING
            || cooldown > COOLDOWN
            || flags & !0b1_1111 != 0
            || (close_overloaded && pressure == 0)
            || (wave_overloaded && (!wave_locked || pressure == 0))
            || wave_locked != (locked_wave_aim != FixedVec2::ZERO)
            || !in_bounds(locked_wave_aim, Fixed::ONE)
            || guard_ticks > GUARD_TICKS
            || dash_ticks > DASH_TICKS
            || dash_cooldown > DASH_COOLDOWN
            || (dash_ticks == 0) != (dash_direction == FixedVec2::ZERO)
            || !in_bounds(dash_direction, Fixed::ONE)
            || !in_bounds(position, ARENA)
            || !in_bounds(opponent_position, ARENA)
            || (phase != Phase::Close && opponent_position != FixedVec2::ZERO)
            || !valid_lunge
            || match phase {
                Phase::Close => {
                    age == tick
                        || target_health > 2
                        || has_projectiles
                        || cooldown != 0
                        || (combo_clean && !(FIRST_STRIKE..final_beat).contains(&(age % HZ)))
                }
                Phase::Shift => {
                    age == tick
                        || age >= SHIFT_TICKS
                        || target_health != 2
                        || has_projectiles
                        || opening != 0
                        || cooldown != 0
                        || combo_clean
                        || close_overloaded
                }
                Phase::Waves => {
                    age != tick
                        || !(1..=3).contains(&target_health)
                        || opening != 0
                        || combo_clean
                        || close_overloaded
                }
            }
        {
            return Err("invalid duel state");
        }

        if !bytes.is_empty() {
            return Err("trailing snapshot bytes");
        }
        Ok(Self {
            tick,
            age,
            phase,
            outcome,
            health,
            target_health,
            charge,
            pressure,
            position,
            opponent_position,
            lunge,
            opening,
            cooldown,
            guard_held: flags & 0b001 != 0,
            guard_ticks,
            combo_clean,
            close_overloaded,
            dash_held: flags & 0b100 != 0,
            dash_ticks,
            dash_cooldown,
            dash_direction,
            locked_wave_aim,
            wave_overloaded,
            bolts,
            shots,
        })
    }
}

fn write_vector(out: &mut impl StateWriter, value: FixedVec2) {
    out.write_i64(value.x.raw());
    out.write_i64(value.y.raw());
}

fn take<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], &'static str> {
    let (value, rest) = bytes.split_first_chunk().ok_or("truncated snapshot")?;
    *bytes = rest;
    Ok(*value)
}

fn read_vector(bytes: &mut &[u8]) -> Result<FixedVec2, &'static str> {
    Ok(FixedVec2::new(
        Fixed::from_raw(i64::from_le_bytes(take(bytes)?)),
        Fixed::from_raw(i64::from_le_bytes(take(bytes)?)),
    ))
}

fn read_projectiles(bytes: &mut &[u8], max: usize) -> Result<Vec<Bolt>, &'static str> {
    let [count] = take(bytes)?;
    if usize::from(count) > max {
        return Err("too many projectiles");
    }
    let mut projectiles = Vec::with_capacity(max);
    for _ in 0..count {
        let position = read_vector(bytes)?;
        let velocity = read_vector(bytes)?;
        if !in_bounds(position, BOLT_BOUND)
            || !in_bounds(velocity, Fixed::ONE)
            || velocity == FixedVec2::ZERO
        {
            return Err("invalid projectile");
        }
        projectiles.push(Bolt { position, velocity });
    }
    Ok(projectiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_to_lock_lunge() -> Duel {
        let mut duel = Duel::new();
        duel.phase = Phase::Close;
        duel.tick = 200;
        duel.age = LUNGE_LOCK - 1;
        duel.target_health = 2;
        duel.position = vec(0, 4);
        duel
    }

    #[test]
    fn sidestepping_the_locked_lunge_opens_a_counter_after_midflight_restore() {
        let mut duel = ready_to_lock_lunge();
        duel.step(Command::default());
        let locked = duel.lunge.unwrap();
        assert_eq!(locked.to, vec(0, 4));
        let right = Command {
            x: FULL_AXIS,
            ..Default::default()
        };
        while duel.age <= LUNGE_START + 1 {
            duel.step(right);
        }
        assert!(duel.opponent_position != locked.from && duel.opponent_position != locked.to);
        let mut restored = Duel::restore(&duel.snapshot()).unwrap();
        while duel.age < LUNGE_END {
            duel.step(Command::default());
            restored.step(Command::default());
            assert_eq!(duel.snapshot(), restored.snapshot());
        }
        assert_eq!(duel.opponent_position, locked.to);
        assert_eq!(duel.health, HEALTH);
        assert_eq!(duel.opening, OPENING);
        duel.step(Command {
            attack: true,
            ..Default::default()
        });
        assert_eq!(duel.target_health, 1);
    }

    #[test]
    fn lunge_sweeps_for_contact_once_and_can_be_parried() {
        let mut hit = ready_to_lock_lunge();
        let mut guard = Duel::restore(&hit.snapshot()).unwrap();
        let mut parried = false;
        // The parry is timed for contact on the way in, not the endpoint.
        while hit.age < LUNGE_END {
            hit.step(Command::default());
            let cue = guard.step(Command {
                guard: guard.age + 1 == LUNGE_END - 2,
                ..Default::default()
            });
            parried |= cue == StepCue::Parry;
        }
        assert_eq!((hit.health, hit.opening), (HEALTH - 1, 0));
        assert!(parried);
        assert_eq!((guard.health, guard.opening), (HEALTH, OPENING));
    }

    #[test]
    fn quantized_movement_preserves_partial_deflection_and_caps_diagonal_speed() {
        let movement = |x, z| {
            let mut duel = Duel::new();
            let before = duel.position;
            duel.step(Command {
                x,
                z,
                ..Default::default()
            });
            duel.position - before
        };
        let full = movement(FULL_AXIS, 0);
        let partial = movement(FULL_AXIS / 2, 0);
        assert_eq!(full.x, Fixed::from_ratio(1, 8));
        assert!(partial.x > Fixed::ZERO && partial.x < full.x);
        assert_eq!(partial.y, Fixed::ZERO);
        assert!(movement(FULL_AXIS, FULL_AXIS).length_squared() <= full.length_squared());
    }

    #[test]
    fn projectile_crossing_is_detected_even_with_both_endpoints_outside() {
        let mut duel = Duel::new();
        duel.phase = Phase::Waves;
        duel.age = 1;
        duel.position = vec(0, 2);
        duel.bolts.push(Bolt {
            position: FixedVec2::new(Fixed::from_ratio(-1, 2), Fixed::from_int(2)),
            velocity: vec(1, 0),
        });
        duel.step(Command::default());
        assert_eq!(duel.health, HEALTH - 1);
        assert!(duel.bolts.is_empty());
    }

    #[test]
    fn inactivity_can_lose_and_terminal_states_do_not_advance() {
        let mut duel = Duel::new();
        while duel.outcome == Outcome::Active {
            duel.step(Command::default());
        }
        assert_eq!(duel.outcome, Outcome::Lost);
        let final_state = duel.snapshot();
        duel.step(Command::default());
        assert_eq!(duel.snapshot(), final_state);
        assert_eq!(Duel::new().outcome, Outcome::Active);
    }

    #[test]
    fn boost_spends_charge_and_adds_a_second_volley() {
        let mut normal = Duel::new();
        normal.tick = WAVE_LOCK - 1;
        normal.age = normal.tick;
        normal.position = vec(0, 5);
        normal.health = 3;
        normal.charge = 2;
        let mut boosted = Duel::restore(&normal.snapshot()).unwrap();
        boosted.step(Command {
            boost: true,
            ..Default::default()
        });
        normal.step(Command::default());
        assert_eq!(
            (boosted.health, boosted.charge, boosted.pressure),
            (5, 0, 1)
        );

        while boosted.age < EXTRA_VOLLEY {
            normal.step(Command::default());
            boosted.step(Command::default());
        }
        assert_eq!(normal.bolts.len(), 8);
        assert_eq!(boosted.bolts.len(), 13);
    }

    #[test]
    fn fan_commits_to_the_telegraphed_direction_and_can_be_sidestepped() {
        let mut moving = Duel::new();
        moving.tick = WAVE_CYCLE + WAVE_LOCK - 1;
        moving.age = moving.tick;
        moving.step(Command::default());
        #[cfg(feature = "windowed")]
        let (preview, preview_count, lead) = moving.wave_warning().expect("windup has a warning");
        let saved = moving.snapshot();
        let mut restored = Duel::restore(&saved).unwrap();
        let mut stationary = Duel::restore(&saved).unwrap();
        let sidestep = Command {
            x: FULL_AXIS,
            ..Default::default()
        };
        while moving.age % WAVE_CYCLE < FIRST_VOLLEY {
            moving.step(sidestep);
            restored.step(sidestep);
            stationary.step(Command::default());
            assert_eq!(moving.snapshot(), restored.snapshot());
        }
        assert_eq!(moving.position, vec(2, 2));
        assert_eq!(moving.bolts.len(), 5);
        #[cfg(feature = "windowed")]
        {
            assert_eq!(lead, FIRST_VOLLEY - WAVE_LOCK);
            assert_eq!(preview_count, moving.bolts.len());
            for (bolt, direction) in moving.bolts.iter().zip(preview) {
                assert_eq!(bolt.velocity, direction * Fixed::from_ratio(1, 5));
            }
        }
        assert_eq!(
            moving.bolts, stationary.bolts,
            "the volley must not follow movement"
        );
        assert_eq!(moving.bolts[2].velocity.x, Fixed::ZERO);
        for _ in 0..12 {
            moving.step(Command::default());
            stationary.step(Command::default());
        }
        assert_eq!(moving.health, HEALTH);
        assert!(stationary.health < HEALTH);
    }

    #[test]
    fn spending_charge_after_lock_does_not_add_an_unannounced_volley() {
        let mut duel = Duel::new();
        duel.position = vec(0, 5);
        duel.health = 3;
        duel.charge = 2;
        for _ in 0..WAVE_LOCK {
            duel.step(Command::default());
        }
        duel.step(Command {
            boost: true,
            ..Default::default()
        });
        while duel.age < EXTRA_VOLLEY {
            duel.step(Command::default());
        }
        assert_eq!(duel.pressure, 1);
        assert_eq!(duel.bolts.len(), 8);
    }

    #[test]
    fn ranged_attack_is_effective_only_in_the_recovery_window() {
        let mut duel = Duel::new();
        let attack = Command {
            attack: true,
            aim_z: -FULL_AXIS,
            ..Default::default()
        };
        assert_eq!(duel.step(attack), StepCue::Shot);
        assert_eq!(duel.step(Command::default()), StepCue::Blocked);
        assert_eq!(duel.target_health, 3);
        assert!(duel.shots.is_empty());
        while duel.age < RANGED_OPEN - 1 {
            duel.step(Command::default());
        }
        assert_eq!(duel.step(attack), StepCue::Shot);
        assert_eq!(duel.target_health, 3, "the shot is still in flight");
        assert_eq!(duel.step(Command::default()), StepCue::Hit);
        assert_eq!(duel.target_health, 2);
        assert_eq!(duel.charge, 1);
    }

    #[test]
    fn firing_sideways_does_not_hit_an_open_target() {
        let mut duel = Duel::new();
        duel.tick = RANGED_OPEN - 1;
        duel.age = duel.tick;
        let sideways = Command {
            attack: true,
            aim_x: FULL_AXIS,
            ..Default::default()
        };
        assert_eq!(duel.step(sideways), StepCue::Shot);
        while !duel.shots.is_empty() {
            duel.step(Command::default());
        }
        assert_eq!(duel.target_health, 3);
        assert_eq!(duel.charge, 0);
    }

    #[test]
    fn holding_parry_after_a_save_does_not_defend_the_second_strike() {
        let mut duel = Duel::new();
        duel.phase = Phase::Close;
        duel.tick = 200;
        duel.age = FIRST_STRIKE - 1;
        duel.target_health = 2;
        let held = Command {
            guard: true,
            ..Default::default()
        };
        assert_eq!(duel.step(held), StepCue::Parry);
        let mut duel = Duel::restore(&duel.snapshot()).unwrap();
        while duel.age < LAST_STRIKE {
            assert_eq!(duel.step(held), StepCue::None);
        }
        assert_eq!(duel.health, HEALTH - 1);
        assert_eq!(duel.opening, 0);
        assert_eq!(duel.charge, 1);
    }

    #[test]
    fn spending_charge_in_a_combo_only_changes_the_next_combo() {
        let mut duel = Duel::new();
        duel.phase = Phase::Close;
        duel.tick = 200;
        duel.age = FIRST_STRIKE - 1;
        duel.target_health = 2;
        duel.health = HEALTH - 2;
        duel.charge = 2;
        let parry = Command {
            guard: true,
            ..Default::default()
        };
        assert_eq!(duel.step(parry), StepCue::Parry);
        duel.step(Command {
            boost: true,
            ..Default::default()
        });
        while duel.age < LAST_STRIKE - 1 {
            duel.step(Command::default());
        }
        assert_eq!(duel.step(parry), StepCue::Parry);
        assert_eq!(duel.opening, OPENING);
        while duel.age < HZ + 1 {
            duel.step(Command::default());
        }
        assert!(duel.close_overloaded);
    }

    #[test]
    fn dash_avoids_swept_contact_and_restores_active_direction() {
        let mut duel = Duel::new();
        duel.bolts.push(Bolt {
            position: FixedVec2::new(Fixed::from_ratio(1, 2), Fixed::from_int(2)),
            velocity: vec(-1, 0),
        });
        let command = Command {
            x: FULL_AXIS,
            dash: true,
            ..Default::default()
        };
        assert_eq!(duel.step(command), StepCue::Dash);
        assert_eq!(duel.health, HEALTH);
        assert_eq!(duel.bolts.len(), 1);
        let mut restored = Duel::restore(&duel.snapshot()).unwrap();
        let opposite = Command {
            x: -FULL_AXIS,
            ..command
        };
        assert_eq!(duel.step(opposite), StepCue::None);
        assert_eq!(restored.step(opposite), StepCue::None);
        assert!(duel.position.x > Fixed::from_ratio(3, 8));
        assert_eq!(duel.snapshot(), restored.snapshot());
        for _ in 0..DASH_COOLDOWN {
            assert_ne!(duel.step(command), StepCue::Dash);
        }
    }

    #[test]
    fn malformed_snapshots_are_rejected() {
        let state = Duel::new().snapshot();
        for end in 0..state.len() {
            assert!(Duel::restore(&state[..end]).is_err());
        }
        let mut bad = state.clone();
        bad[4] = 2;
        assert!(Duel::restore(&bad).is_err());
        let mut bad = state.clone();
        bad[16] = HEALTH + 1;
        assert!(Duel::restore(&bad).is_err());
        let mut bad = state;
        bad.push(0);
        assert!(Duel::restore(&bad).is_err());
    }
}
