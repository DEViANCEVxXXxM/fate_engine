//! Fixed-step, deterministic first-person combat simulation.
//!
//! The simulation owns all gameplay rules and uses only `fate_sim` fixed-point
//! math, so it runs headless and produces identical state for identical input.
//! Rendering, input mapping, and presentation live in the other modules.

use std::fmt;

use fate_sim::math::{Fixed, FixedVec2, FixedVec3};
use fate_sim::{StateHasher, StateWriter};

pub const SIMULATION_HZ: u32 = 120;
pub const MAX_HEALTH: i32 = 100;
pub const MAGAZINE_SIZE: u8 = 10;
pub const ENEMY_HEALTH: i32 = 30;
pub const TIME_LIMIT_TICKS: u64 = 90 * SIMULATION_HZ as u64;

/// Largest mouse count accepted in one fixed step.
///
/// The app clamps raw mouse motion to this range so a single step can never
/// rotate the view by an unreasonable angle.
pub const MAX_LOOK_INPUT: i16 = 4096;

const STATE_FORMAT_VERSION: u16 = 1;
const STATE_MAGIC: &[u8; 4] = b"FCS1";
const MAX_ENEMIES: u32 = 32;

const FIXED_DELTA: Fixed = Fixed::from_ratio(1, SIMULATION_HZ as i64);
const ARENA_HALF: Fixed = Fixed::from_int(12);
const PLAYER_RADIUS: Fixed = Fixed::from_ratio(2, 5);
pub const PLAYER_EYE_HEIGHT: Fixed = Fixed::from_ratio(8, 5);
const PLAYER_SPEED: Fixed = Fixed::from_int(6);
const PLAYER_SPRINT_SPEED: Fixed = Fixed::from_int(9);
const PLAYER_ACCEL: Fixed = Fixed::from_ratio(1, 6);

const RELOAD_TICKS: u32 = 84;
const FIRE_COOLDOWN_TICKS: u32 = 15;
const SHOT_DAMAGE: i32 = 18;

const ENEMY_RADIUS: Fixed = Fixed::from_ratio(4, 5);
pub const ENEMY_CENTER_Y: Fixed = Fixed::ONE;
const ENEMY_SPEED: Fixed = Fixed::from_int(2);
const CONTACT_RADIUS: Fixed = Fixed::from_ratio(6, 5);
const CONTACT_DAMAGE: i32 = 8;
const CONTACT_COOLDOWN_TICKS: u32 = 36;

const LOOK_RADIANS_PER_COUNT: Fixed = Fixed::from_ratio(1, 400);
const MAX_LOOK_STEP: Fixed = Fixed::from_ratio(1, 4);
const MAX_PITCH_SIN: Fixed = Fixed::from_ratio(17, 20);

/// A view direction stored as unit vectors.
///
/// `yaw` is `(cos yaw, sin yaw)` in the horizontal plane and `pitch` is
/// `(cos pitch, sin pitch)`. Keeping both as vectors avoids needing fixed-point
/// trigonometric functions in the hot path: a small mouse delta is applied with
/// short Taylor polynomials and the result is renormalized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    yaw: FixedVec2,
    pitch: FixedVec2,
}

impl Look {
    pub const fn level() -> Self {
        Self {
            yaw: FixedVec2::new(Fixed::ONE, Fixed::ZERO),
            pitch: FixedVec2::new(Fixed::ONE, Fixed::ZERO),
        }
    }

    pub const fn yaw_vector(self) -> FixedVec2 {
        self.yaw
    }

    pub const fn pitch_vector(self) -> FixedVec2 {
        self.pitch
    }

    /// Horizontal unit direction `(x, z)`.
    pub fn forward_horizontal(self) -> FixedVec2 {
        FixedVec2::new(self.yaw.y, -self.yaw.x)
    }

    /// Horizontal unit right direction `(x, z)`.
    pub const fn right_horizontal(self) -> FixedVec2 {
        FixedVec2::new(self.yaw.x, self.yaw.y)
    }

    /// Full unit aim direction in world space.
    pub fn forward(self) -> FixedVec3 {
        FixedVec3::new(
            self.yaw.y * self.pitch.x,
            self.pitch.y,
            -(self.yaw.x * self.pitch.x),
        )
    }

    /// Apply a horizontal and vertical rotation in radians.
    pub fn apply(&mut self, yaw_delta: Fixed, pitch_delta: Fixed) {
        self.yaw = rotate_unit(self.yaw, yaw_delta);
        self.pitch = clamp_pitch(rotate_unit(self.pitch, pitch_delta));
    }

    /// Renderer-facing yaw in radians.
    pub fn yaw_angle(self) -> f32 {
        self.yaw.y.to_f32().atan2(self.yaw.x.to_f32())
    }

    /// Renderer-facing pitch in radians.
    pub fn pitch_angle(self) -> f32 {
        self.pitch.y.to_f32().clamp(-1.0, 1.0).asin()
    }
}

impl Default for Look {
    fn default() -> Self {
        Self::level()
    }
}

/// One fixed-step input command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimInput {
    /// Strafe axis in `-1..=1`.
    pub move_x: i8,
    /// Forward axis in `-1..=1`.
    pub move_y: i8,
    /// Horizontal look delta in mouse counts.
    pub look_yaw: i16,
    /// Vertical look delta in mouse counts.
    pub look_pitch: i16,
    pub sprint: bool,
    pub fire: bool,
    pub reload: bool,
}

impl SimInput {
    /// Clamp every axis into its accepted range.
    pub fn sanitized(self) -> Self {
        Self {
            move_x: self.move_x.clamp(-1, 1),
            move_y: self.move_y.clamp(-1, 1),
            look_yaw: self.look_yaw.clamp(-MAX_LOOK_INPUT, MAX_LOOK_INPUT),
            look_pitch: self.look_pitch.clamp(-MAX_LOOK_INPUT, MAX_LOOK_INPUT),
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerState {
    /// World position on the horizontal plane; `y` is the world Z axis.
    pub position: FixedVec2,
    pub velocity: FixedVec2,
    pub look: Look,
    pub health: i32,
    pub ammo: u8,
    pub reload_remaining: u32,
    pub fire_cooldown: u32,
}

impl PlayerState {
    pub fn alive(self) -> bool {
        self.health > 0
    }

    pub fn eye_position(self) -> FixedVec3 {
        FixedVec3::new(self.position.x, PLAYER_EYE_HEIGHT, self.position.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnemyState {
    pub position: FixedVec2,
    pub health: i32,
    pub attack_cooldown: u32,
}

impl EnemyState {
    pub fn alive(self) -> bool {
        self.health > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefeatReason {
    HealthDepleted,
    TimeExpired,
}

impl DefeatReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::HealthDepleted => "health depleted",
            Self::TimeExpired => "time expired",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    Running,
    Victory,
    Defeat(DefeatReason),
}

impl Outcome {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Victory => "victory",
            Self::Defeat(reason) => reason.label(),
        }
    }

    pub const fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateDecodeError {
    BadMagic,
    Truncated,
    UnsupportedVersion(u16),
    InvalidOutcome(u8),
    InvalidLastShot(u8),
    InvalidPlayer,
    InvalidEnemyCount(u32),
    InvalidEnemy,
    TrailingBytes,
}

impl fmt::Display for StateDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => formatter.write_str("invalid simulation snapshot signature"),
            Self::Truncated => formatter.write_str("truncated simulation snapshot"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported simulation snapshot version {version}"
                )
            }
            Self::InvalidOutcome(code) => write!(formatter, "invalid outcome code {code}"),
            Self::InvalidLastShot(flag) => {
                write!(formatter, "invalid last-shot flag {flag} in snapshot")
            }
            Self::InvalidPlayer => formatter.write_str("invalid player state in snapshot"),
            Self::InvalidEnemyCount(count) => {
                write!(formatter, "invalid enemy count {count} in snapshot")
            }
            Self::InvalidEnemy => formatter.write_str("invalid enemy state in snapshot"),
            Self::TrailingBytes => formatter.write_str("unexpected trailing bytes in snapshot"),
        }
    }
}

impl std::error::Error for StateDecodeError {}

/// Complete deterministic combat state.
pub struct Sim {
    pub player: PlayerState,
    pub enemies: Vec<EnemyState>,
    tick: u64,
    outcome: Outcome,
    shots_fired: u32,
    hits: u32,
    kills: u32,
    last_shot_tick: Option<u64>,
}

impl Sim {
    pub fn new() -> Self {
        let spawns = [
            FixedVec2::new(Fixed::ZERO, Fixed::from_int(-7)),
            FixedVec2::new(Fixed::from_int(-5), Fixed::from_int(-3)),
            FixedVec2::new(Fixed::from_int(5), Fixed::from_int(-3)),
            FixedVec2::new(Fixed::from_int(-8), Fixed::from_int(3)),
            FixedVec2::new(Fixed::from_int(8), Fixed::from_int(3)),
            FixedVec2::new(Fixed::ZERO, Fixed::ZERO),
        ];
        let enemies = spawns
            .into_iter()
            .enumerate()
            .map(|(index, position)| EnemyState {
                position,
                health: ENEMY_HEALTH,
                attack_cooldown: index as u32 * 5,
            })
            .collect();

        Self {
            player: PlayerState {
                position: FixedVec2::new(Fixed::ZERO, Fixed::from_int(7)),
                velocity: FixedVec2::ZERO,
                look: Look::level(),
                health: MAX_HEALTH,
                ammo: MAGAZINE_SIZE,
                reload_remaining: 0,
                fire_cooldown: 0,
            },
            enemies,
            tick: 0,
            outcome: Outcome::Running,
            shots_fired: 0,
            hits: 0,
            kills: 0,
            last_shot_tick: None,
        }
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn shots_fired(&self) -> u32 {
        self.shots_fired
    }

    pub fn hits(&self) -> u32 {
        self.hits
    }

    pub fn kills(&self) -> u32 {
        self.kills
    }

    pub fn last_shot_tick(&self) -> Option<u64> {
        self.last_shot_tick
    }

    pub fn enemies_alive(&self) -> usize {
        self.enemies.iter().filter(|enemy| enemy.alive()).count()
    }

    /// Advance exactly one fixed step.
    pub fn step(&mut self, input: SimInput) {
        if !self.outcome.is_running() {
            return;
        }

        let input = input.sanitized();
        self.tick = self.tick.saturating_add(1);

        let yaw_delta = Fixed::from_int(input.look_yaw as i64) * LOOK_RADIANS_PER_COUNT;
        let pitch_delta = Fixed::from_int(input.look_pitch as i64) * LOOK_RADIANS_PER_COUNT;
        self.player.look.apply(yaw_delta, pitch_delta);

        self.move_player(&input);
        self.update_timers(&input);
        if input.fire
            && self.player.ammo > 0
            && self.player.fire_cooldown == 0
            && self.player.reload_remaining == 0
        {
            self.fire();
        }
        self.update_enemies();
        self.update_outcome();
    }

    fn move_player(&mut self, input: &SimInput) {
        let direction = self.player.look.forward_horizontal()
            * Fixed::from_int(input.move_y as i64)
            + self.player.look.right_horizontal() * Fixed::from_int(input.move_x as i64);
        let direction = if direction.length_squared() > Fixed::ONE {
            direction.normalize()
        } else {
            direction
        };

        let speed = if input.sprint {
            PLAYER_SPRINT_SPEED
        } else {
            PLAYER_SPEED
        };
        let target = direction * speed;
        self.player.velocity.x += (target.x - self.player.velocity.x) * PLAYER_ACCEL;
        self.player.velocity.y += (target.y - self.player.velocity.y) * PLAYER_ACCEL;
        self.player.position += self.player.velocity * FIXED_DELTA;

        let limit = ARENA_HALF - PLAYER_RADIUS;
        if self.player.position.x > limit || self.player.position.x < -limit {
            self.player.position.x = self.player.position.x.clamp(-limit, limit);
            self.player.velocity.x = Fixed::ZERO;
        }
        if self.player.position.y > limit || self.player.position.y < -limit {
            self.player.position.y = self.player.position.y.clamp(-limit, limit);
            self.player.velocity.y = Fixed::ZERO;
        }
    }

    fn update_timers(&mut self, input: &SimInput) {
        self.player.fire_cooldown = self.player.fire_cooldown.saturating_sub(1);
        if self.player.reload_remaining > 0 {
            self.player.reload_remaining -= 1;
            if self.player.reload_remaining == 0 {
                self.player.ammo = MAGAZINE_SIZE;
            }
        } else if input.reload && self.player.ammo < MAGAZINE_SIZE {
            self.player.reload_remaining = RELOAD_TICKS;
        }
    }

    fn fire(&mut self) {
        if self.player.ammo == 0 {
            return;
        }
        self.player.ammo -= 1;
        self.player.fire_cooldown = FIRE_COOLDOWN_TICKS;
        self.shots_fired = self.shots_fired.saturating_add(1);
        self.last_shot_tick = Some(self.tick);

        let origin = self.player.eye_position();
        let direction = self.player.look.forward();
        let mut nearest: Option<(usize, Fixed)> = None;
        for (index, enemy) in self.enemies.iter().enumerate() {
            if !enemy.alive() {
                continue;
            }
            let center = FixedVec3::new(enemy.position.x, ENEMY_CENTER_Y, enemy.position.y);
            let Some(distance) = ray_sphere(origin, direction, center, ENEMY_RADIUS) else {
                continue;
            };
            if nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some((index, distance));
            }
        }

        let Some((index, _)) = nearest else {
            return;
        };
        self.hits = self.hits.saturating_add(1);
        let enemy = &mut self.enemies[index];
        enemy.health = (enemy.health - SHOT_DAMAGE).max(0);
        if !enemy.alive() {
            self.kills = self.kills.saturating_add(1);
        }
    }

    fn update_enemies(&mut self) {
        let player_position = self.player.position;
        for enemy in &mut self.enemies {
            if !enemy.alive() {
                continue;
            }

            enemy.attack_cooldown = enemy.attack_cooldown.saturating_sub(1);
            let offset = player_position - enemy.position;
            let distance = offset.length();
            if distance > CONTACT_RADIUS {
                let direction = if distance == Fixed::ZERO {
                    FixedVec2::ZERO
                } else {
                    offset / distance
                };
                enemy.position += direction * (ENEMY_SPEED * FIXED_DELTA);
                let limit = ARENA_HALF - ENEMY_RADIUS;
                enemy.position.x = enemy.position.x.clamp(-limit, limit);
                enemy.position.y = enemy.position.y.clamp(-limit, limit);
            }

            if (player_position - enemy.position).length() <= CONTACT_RADIUS
                && enemy.attack_cooldown == 0
            {
                self.player.health -= CONTACT_DAMAGE;
                enemy.attack_cooldown = CONTACT_COOLDOWN_TICKS;
            }
        }
    }

    fn update_outcome(&mut self) {
        if !self.player.alive() {
            self.player.health = 0;
            self.outcome = Outcome::Defeat(DefeatReason::HealthDepleted);
        } else if self.enemies_alive() == 0 {
            self.outcome = Outcome::Victory;
        } else if self.tick >= TIME_LIMIT_TICKS {
            self.outcome = Outcome::Defeat(DefeatReason::TimeExpired);
        }
    }

    /// Stream canonical Little-Endian state bytes into any `StateWriter` sink.
    pub fn write_state<W: StateWriter>(&self, writer: &mut W) {
        writer.write_bytes(STATE_MAGIC);
        writer.write_u16(STATE_FORMAT_VERSION);
        writer.write_u64(self.tick);
        writer.write_u8(outcome_code(self.outcome));
        writer.write_u32(self.shots_fired);
        writer.write_u32(self.hits);
        writer.write_u32(self.kills);
        match self.last_shot_tick {
            Some(tick) => {
                writer.write_u8(1);
                writer.write_u64(tick);
            }
            None => writer.write_u8(0),
        }

        writer.write_i64(self.player.position.x.raw());
        writer.write_i64(self.player.position.y.raw());
        writer.write_i64(self.player.velocity.x.raw());
        writer.write_i64(self.player.velocity.y.raw());
        writer.write_i64(self.player.look.yaw_vector().x.raw());
        writer.write_i64(self.player.look.yaw_vector().y.raw());
        writer.write_i64(self.player.look.pitch_vector().x.raw());
        writer.write_i64(self.player.look.pitch_vector().y.raw());
        writer.write_i64(self.player.health as i64);
        writer.write_u8(self.player.ammo);
        writer.write_u32(self.player.reload_remaining);
        writer.write_u32(self.player.fire_cooldown);

        writer.write_u32(self.enemies.len() as u32);
        for enemy in &self.enemies {
            writer.write_i64(enemy.position.x.raw());
            writer.write_i64(enemy.position.y.raw());
            writer.write_i64(enemy.health as i64);
            writer.write_u32(enemy.attack_cooldown);
        }
    }

    /// Versioned canonical snapshot.
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.write_state(&mut bytes);
        bytes
    }

    pub fn from_state_bytes(bytes: &[u8]) -> Result<Self, StateDecodeError> {
        let mut offset = 0;
        if Self::read_bytes::<4>(bytes, &mut offset)? != *STATE_MAGIC {
            return Err(StateDecodeError::BadMagic);
        }
        let version = Self::read_u16(bytes, &mut offset)?;
        if version != STATE_FORMAT_VERSION {
            return Err(StateDecodeError::UnsupportedVersion(version));
        }

        let tick = Self::read_u64(bytes, &mut offset)?;
        let outcome_code = Self::read_u8(bytes, &mut offset)?;
        let outcome = outcome_from_code(outcome_code)
            .ok_or(StateDecodeError::InvalidOutcome(outcome_code))?;
        let shots_fired = Self::read_u32(bytes, &mut offset)?;
        let hits = Self::read_u32(bytes, &mut offset)?;
        let kills = Self::read_u32(bytes, &mut offset)?;
        let last_shot_tick = match Self::read_u8(bytes, &mut offset)? {
            0 => None,
            1 => Some(Self::read_u64(bytes, &mut offset)?),
            flag => return Err(StateDecodeError::InvalidLastShot(flag)),
        };

        let position = FixedVec2::new(
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
        );
        let velocity = FixedVec2::new(
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
        );
        let yaw = FixedVec2::new(
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
        );
        let pitch = FixedVec2::new(
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
            Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
        );
        let health = Self::read_i64(bytes, &mut offset)?;
        let ammo = Self::read_u8(bytes, &mut offset)?;
        let reload_remaining = Self::read_u32(bytes, &mut offset)?;
        let fire_cooldown = Self::read_u32(bytes, &mut offset)?;

        if !(0..=MAX_HEALTH as i64).contains(&health)
            || ammo > MAGAZINE_SIZE
            || yaw.length_squared() == Fixed::ZERO
            || pitch.length_squared() == Fixed::ZERO
        {
            return Err(StateDecodeError::InvalidPlayer);
        }

        let enemy_count = Self::read_u32(bytes, &mut offset)?;
        if enemy_count > MAX_ENEMIES {
            return Err(StateDecodeError::InvalidEnemyCount(enemy_count));
        }
        let mut enemies = Vec::with_capacity(enemy_count as usize);
        for _ in 0..enemy_count {
            let position = FixedVec2::new(
                Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
                Fixed::from_raw(Self::read_i64(bytes, &mut offset)?),
            );
            let health = Self::read_i64(bytes, &mut offset)?;
            let attack_cooldown = Self::read_u32(bytes, &mut offset)?;
            if !(0..=ENEMY_HEALTH as i64).contains(&health) {
                return Err(StateDecodeError::InvalidEnemy);
            }
            enemies.push(EnemyState {
                position,
                health: health as i32,
                attack_cooldown,
            });
        }

        if offset != bytes.len() {
            return Err(StateDecodeError::TrailingBytes);
        }

        Ok(Self {
            player: PlayerState {
                position,
                velocity,
                look: Look { yaw, pitch },
                health: health as i32,
                ammo,
                reload_remaining,
                fire_cooldown,
            },
            enemies,
            tick,
            outcome,
            shots_fired,
            hits,
            kills,
            last_shot_tick,
        })
    }

    /// FNV-1a 64-bit diagnostic hash over the canonical state bytes.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        self.write_state(&mut hasher);
        hasher.finish()
    }

    fn read_bytes<const N: usize>(
        bytes: &[u8],
        offset: &mut usize,
    ) -> Result<[u8; N], StateDecodeError> {
        let end = offset.checked_add(N).ok_or(StateDecodeError::Truncated)?;
        let value = bytes
            .get(*offset..end)
            .ok_or(StateDecodeError::Truncated)?
            .try_into()
            .expect("fixed-size slice");
        *offset = end;
        Ok(value)
    }

    fn read_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, StateDecodeError> {
        Ok(Self::read_bytes::<1>(bytes, offset)?[0])
    }

    fn read_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, StateDecodeError> {
        Ok(u16::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }

    fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, StateDecodeError> {
        Ok(u32::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }

    fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, StateDecodeError> {
        Ok(u64::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }

    fn read_i64(bytes: &[u8], offset: &mut usize) -> Result<i64, StateDecodeError> {
        Ok(i64::from_le_bytes(Self::read_bytes(bytes, offset)?))
    }
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

fn outcome_code(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Running => 0,
        Outcome::Victory => 1,
        Outcome::Defeat(DefeatReason::HealthDepleted) => 2,
        Outcome::Defeat(DefeatReason::TimeExpired) => 3,
    }
}

fn outcome_from_code(code: u8) -> Option<Outcome> {
    match code {
        0 => Some(Outcome::Running),
        1 => Some(Outcome::Victory),
        2 => Some(Outcome::Defeat(DefeatReason::HealthDepleted)),
        3 => Some(Outcome::Defeat(DefeatReason::TimeExpired)),
        _ => None,
    }
}

fn sin_small(angle: Fixed) -> Fixed {
    let square = angle * angle;
    angle * (Fixed::ONE - square * (Fixed::from_ratio(1, 6) - square * Fixed::from_ratio(1, 120)))
}

fn cos_small(angle: Fixed) -> Fixed {
    let square = angle * angle;
    Fixed::ONE - square * (Fixed::from_ratio(1, 2) - square * Fixed::from_ratio(1, 24))
}

fn rotate_unit(vector: FixedVec2, angle: Fixed) -> FixedVec2 {
    let angle = angle.clamp(-MAX_LOOK_STEP, MAX_LOOK_STEP);
    let sin = sin_small(angle);
    let cos = cos_small(angle);
    FixedVec2::new(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
    )
    .normalize()
}

fn clamp_pitch(pitch: FixedVec2) -> FixedVec2 {
    let y = pitch.y.clamp(-MAX_PITCH_SIN, MAX_PITCH_SIN);
    let x = (Fixed::ONE - y * y).sqrt();
    FixedVec2::new(x, y)
}

fn ray_sphere(
    origin: FixedVec3,
    direction: FixedVec3,
    center: FixedVec3,
    radius: Fixed,
) -> Option<Fixed> {
    let offset = center - origin;
    let along = offset.dot(direction);
    if along < Fixed::ZERO {
        return None;
    }
    let perpendicular_squared = offset.length_squared() - along * along;
    let radius_squared = radius * radius;
    if perpendicular_squared > radius_squared {
        return None;
    }
    let half_chord = (radius_squared - perpendicular_squared).sqrt();
    let near = along - half_chord;
    if near >= Fixed::ZERO {
        Some(near)
    } else {
        Some(along + half_chord)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strafing_input(tick: u64) -> SimInput {
        SimInput {
            move_x: match (tick / 30) % 3 {
                0 => 1,
                1 => 0,
                _ => -1,
            },
            move_y: if tick % 90 < 45 { 1 } else { -1 },
            look_yaw: ((tick % 40) as i16 - 20) * 3,
            look_pitch: ((tick % 20) as i16 - 10) * 2,
            sprint: tick.is_multiple_of(60),
            fire: false,
            reload: false,
        }
    }

    #[test]
    fn initial_state_is_running_and_populated() {
        let sim = Sim::new();

        assert_eq!(sim.tick(), 0);
        assert_eq!(sim.outcome(), Outcome::Running);
        assert_eq!(sim.player.health, MAX_HEALTH);
        assert_eq!(sim.player.ammo, MAGAZINE_SIZE);
        assert_eq!(sim.enemies_alive(), sim.enemies.len());
        assert_eq!(sim.state_bytes().len(), Sim::new().state_bytes().len());
    }

    #[test]
    fn movement_is_clamped_to_the_arena() {
        let mut sim = Sim::new();
        for _ in 0..2_000 {
            sim.step(SimInput {
                move_y: 1,
                sprint: true,
                ..SimInput::default()
            });
        }

        let limit = ARENA_HALF - PLAYER_RADIUS;
        assert!(sim.player.position.x <= limit && sim.player.position.x >= -limit);
        assert!(sim.player.position.y <= limit && sim.player.position.y >= -limit);
        assert_eq!(sim.player.velocity.y, Fixed::ZERO);
    }

    #[test]
    fn look_rotation_keeps_unit_vectors_and_limits_pitch() {
        let mut sim = Sim::new();
        for _ in 0..1_000 {
            sim.step(SimInput {
                look_yaw: MAX_LOOK_INPUT,
                look_pitch: -MAX_LOOK_INPUT,
                ..SimInput::default()
            });
        }

        let yaw = sim.player.look.yaw_vector();
        let pitch = sim.player.look.pitch_vector();
        assert!(yaw.length().raw().abs_diff(Fixed::ONE.raw()) <= 4);
        assert!(pitch.length().raw().abs_diff(Fixed::ONE.raw()) <= 4);
        assert!(pitch.y >= -MAX_PITCH_SIN && pitch.y <= MAX_PITCH_SIN);
        assert!(pitch.x >= Fixed::ZERO);
    }

    #[test]
    fn hitscan_damages_the_enemy_in_front() {
        let mut sim = Sim::new();
        sim.enemies[5].position = FixedVec2::new(Fixed::ZERO, Fixed::ZERO);
        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });

        assert_eq!(sim.shots_fired(), 1);
        assert_eq!(sim.hits(), 1);
        assert_eq!(sim.enemies[5].health, ENEMY_HEALTH - SHOT_DAMAGE);
        assert_eq!(sim.player.ammo, MAGAZINE_SIZE - 1);
    }

    #[test]
    fn shots_miss_when_facing_away() {
        let mut sim = Sim::new();
        sim.player.look = Look {
            yaw: FixedVec2::new(-Fixed::ONE, Fixed::ZERO),
            pitch: FixedVec2::new(Fixed::ONE, Fixed::ZERO),
        };
        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });

        assert_eq!(sim.shots_fired(), 1);
        assert_eq!(sim.hits(), 0);
        assert!(sim.enemies.iter().all(|enemy| enemy.health == ENEMY_HEALTH));
    }

    #[test]
    fn repeated_hits_kill_enemies_and_clear_the_arena() {
        let mut sim = Sim::new();
        for enemy in &mut sim.enemies {
            enemy.position = FixedVec2::new(Fixed::ZERO, Fixed::ZERO);
            enemy.health = 1;
        }

        for _ in 0..200 {
            sim.step(SimInput {
                fire: true,
                ..SimInput::default()
            });
            if !sim.outcome().is_running() {
                break;
            }
        }

        assert_eq!(sim.enemies_alive(), 0);
        assert_eq!(sim.outcome(), Outcome::Victory);
        assert_eq!(sim.kills(), sim.enemies.len() as u32);
    }

    #[test]
    fn reload_refills_the_magazine_after_the_delay() {
        let mut sim = Sim::new();
        sim.player.ammo = 1;
        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });
        assert_eq!(sim.player.ammo, 0);

        sim.step(SimInput {
            reload: true,
            ..SimInput::default()
        });
        assert!(sim.player.reload_remaining > 0);

        for _ in 0..RELOAD_TICKS {
            sim.step(SimInput::default());
        }
        assert_eq!(sim.player.ammo, MAGAZINE_SIZE);
        assert_eq!(sim.player.reload_remaining, 0);
    }

    #[test]
    fn firing_with_an_empty_magazine_does_not_underflow() {
        let mut sim = Sim::new();
        sim.player.ammo = 0;

        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });

        assert_eq!(sim.player.ammo, 0);
        assert_eq!(sim.shots_fired(), 0);
        assert_eq!(sim.hits(), 0);
        assert_eq!(sim.player.fire_cooldown, 0);

        // The empty click must not block a later reload and shot.
        sim.step(SimInput {
            reload: true,
            ..SimInput::default()
        });
        for _ in 0..RELOAD_TICKS {
            sim.step(SimInput::default());
        }
        assert_eq!(sim.player.ammo, MAGAZINE_SIZE);

        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });
        assert_eq!(sim.player.ammo, MAGAZINE_SIZE - 1);
        assert_eq!(sim.shots_fired(), 1);
    }

    #[test]
    fn contact_damage_causes_defeat() {
        let mut sim = Sim::new();
        sim.player.health = CONTACT_DAMAGE;
        sim.enemies[0].position = sim.player.position;
        sim.enemies[0].attack_cooldown = 0;
        sim.step(SimInput::default());

        assert_eq!(sim.player.health, 0);
        assert_eq!(sim.outcome(), Outcome::Defeat(DefeatReason::HealthDepleted));
    }

    #[test]
    fn time_limit_causes_defeat() {
        let mut sim = Sim::new();
        sim.tick = TIME_LIMIT_TICKS - 1;
        for enemy in &mut sim.enemies {
            enemy.position = FixedVec2::new(Fixed::from_int(11), Fixed::from_int(11));
        }
        sim.step(SimInput::default());

        assert_eq!(sim.outcome(), Outcome::Defeat(DefeatReason::TimeExpired));
    }

    #[test]
    fn identical_inputs_produce_identical_state() {
        let mut first = Sim::new();
        let mut second = Sim::new();

        for tick in 0..600 {
            let input = strafing_input(tick);
            first.step(input);
            second.step(input);
            assert_eq!(first.state_hash(), second.state_hash(), "tick {tick}");
        }

        assert_eq!(first.state_bytes(), second.state_bytes());
    }

    #[test]
    fn snapshot_round_trip_preserves_state_and_continues_deterministically() {
        let mut original = Sim::new();
        for tick in 0..180 {
            original.step(strafing_input(tick));
        }

        let restored = Sim::from_state_bytes(&original.state_bytes()).unwrap();
        assert_eq!(restored.state_bytes(), original.state_bytes());
        assert_eq!(restored.tick(), 180);

        let mut restored = restored;
        for tick in 180..360 {
            let input = strafing_input(tick);
            original.step(input);
            restored.step(input);
            assert_eq!(restored.state_hash(), original.state_hash(), "tick {tick}");
        }
    }

    #[test]
    fn snapshot_decoder_rejects_malformed_and_trailing_data() {
        let snapshot = Sim::new().state_bytes();
        for length in 0..snapshot.len() {
            assert!(Sim::from_state_bytes(&snapshot[..length]).is_err());
        }

        let mut bad_magic = snapshot.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            Sim::from_state_bytes(&bad_magic),
            Err(StateDecodeError::BadMagic)
        ));

        let mut bad_version = snapshot.clone();
        bad_version[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(matches!(
            Sim::from_state_bytes(&bad_version),
            Err(StateDecodeError::UnsupportedVersion(u16::MAX))
        ));

        let mut trailing = snapshot;
        trailing.push(0);
        assert!(matches!(
            Sim::from_state_bytes(&trailing),
            Err(StateDecodeError::TrailingBytes)
        ));
    }

    #[test]
    fn sanitized_input_clamps_axes() {
        let input = SimInput {
            move_x: 9,
            move_y: -9,
            look_yaw: i16::MAX,
            look_pitch: i16::MIN,
            ..SimInput::default()
        }
        .sanitized();

        assert_eq!(input.move_x, 1);
        assert_eq!(input.move_y, -1);
        assert_eq!(input.look_yaw, MAX_LOOK_INPUT);
        assert_eq!(input.look_pitch, -MAX_LOOK_INPUT);
    }

    #[test]
    fn golden_state_hash_is_stable() {
        let mut sim = Sim::new();
        for tick in 0..240 {
            sim.step(strafing_input(tick));
        }

        assert_eq!(sim.state_hash(), 0x5f26_eefc_7bdb_0776);
    }
}
