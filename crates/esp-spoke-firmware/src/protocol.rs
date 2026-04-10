use crate::{
    bitmap::{BitmapError, BitmapStorage},
    led::{LedError, LedStrip},
};
use pov_proto::transfer::{Packet, ParseError, SpokeCommand, parse_packet};

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub enum CommandApplyResult {
    DisplayOff,
    NextImage { new_index: usize },
}

#[derive(Debug, defmt::Format)]
pub enum CommandApplyError {
    Decode(ParseError),
    Bitmap(BitmapError),
    Led(LedError),
    EmptyStorage,
}

impl From<BitmapError> for CommandApplyError {
    fn from(value: BitmapError) -> Self {
        Self::Bitmap(value)
    }
}

impl From<LedError> for CommandApplyError {
    fn from(value: LedError) -> Self {
        Self::Led(value)
    }
}

/// Decode a raw protocol packet as a command and apply it to the current
/// display state.
///
/// Returns `Ok(None)` when the packet is a non-command message (for example,
/// an image transfer chunk) so callers can hand it to another receive path.
pub fn receive_and_apply_command<S: BitmapStorage, L: LedStrip>(
    raw_packet: &[u8],
    storage: &S,
    current_image: &mut usize,
    led_strip: &mut L,
    target_width: usize,
    target_height: usize,
) -> Result<Option<CommandApplyResult>, CommandApplyError> {
    let command = match parse_packet(raw_packet) {
        Ok(Packet::Command(frame)) => frame.command,
        Ok(Packet::Download(_)) => return Ok(None),
        Err(error) => return Err(CommandApplyError::Decode(error)),
    };

    let result = match command {
        SpokeCommand::DisplayOff => {
            led_strip.clear();
            led_strip.show()?;
            CommandApplyResult::DisplayOff
        }
        SpokeCommand::NextImage => {
            let count = storage.bitmap_count();
            if count == 0 {
                return Err(CommandApplyError::EmptyStorage);
            }

            let next_index = (*current_image + 1) % count;
            let bitmap = storage.bitmap(next_index)?;
            bitmap.scale_into(target_width, target_height, led_strip.pixels_mut())?;
            led_strip.show()?;
            *current_image = next_index;
            CommandApplyResult::NextImage {
                new_index: next_index,
            }
        }
    };

    Ok(Some(result))
}
