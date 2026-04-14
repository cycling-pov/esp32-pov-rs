use std::ops::Rem;

#[derive(Debug, Clone)]
pub struct RotationState {
    pub rotation_rate: f32,
    spoke_positions: Vec<SpokePosition>,
}

#[derive(Debug, Default, Clone, Copy)]
struct SpokePosition {
    current_pos: f32,
    previous_pos: f32,
}

impl RotationState {
    const FULL_CIRCLE: f32 = 2.0 * ::core::f32::consts::PI;

    pub fn new(num_spokes: usize, init_rate: f32) -> Self {
        assert!(num_spokes > 0);
        let mut s = Self {
            rotation_rate: init_rate,
            spoke_positions: vec![SpokePosition::default(); num_spokes],
        };
        s.reset();
        s
    }

    pub fn reset(&mut self) {
        let offset = self.offset_angle();
        for (i, pos) in self.spoke_positions.iter_mut().enumerate() {
            let current = offset * (i as f32);
            pos.current_pos = current;
            pos.previous_pos = current;
        }
    }

    pub fn step(&mut self, dt: f32) {
        let angle_offset = self.offset_angle();

        let init_angle = if let Some(pos) = self.spoke_positions.first_mut() {
            pos.previous_pos = pos.current_pos;
            pos.current_pos = (pos.current_pos + dt * self.rotation_rate).rem(Self::FULL_CIRCLE);
            pos.current_pos
        } else {
            panic!("unable to get spoke values");
        };

        for pos in &mut self.spoke_positions[1..] {
            pos.previous_pos = pos.current_pos;
            pos.current_pos = (init_angle + angle_offset).rem(Self::FULL_CIRCLE);
        }
    }

    fn offset_angle(&self) -> f32 {
        let num_spokes = self.spoke_positions.len();
        assert!(num_spokes > 0);
        Self::FULL_CIRCLE / num_spokes as f32
    }

    pub fn contains(&self, x: f32) -> bool {
        for spoke in &self.spoke_positions {
            let res = if spoke.current_pos > spoke.previous_pos {
                x >= spoke.previous_pos && x <= spoke.current_pos
            } else {
                x <= spoke.current_pos || x >= spoke.previous_pos
            };

            if res {
                return true;
            }
        }

        false
    }

    pub fn has_rotated(&self) -> bool {
        self.spoke_positions[0].previous_pos > self.spoke_positions[0].current_pos
    }
}
