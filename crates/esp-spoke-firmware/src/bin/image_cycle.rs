use defmt::{info, warn};
use embassy_time::Duration;
use esp_spoke_firmware::led::{self, LedCommand};
use esp_spoke_firmware::pushbutton::{self, ButtonId};
use esp_spoke_firmware::storage;

fn select_cycled_slot(
    image_ids: &[usize],
    current_slot: Option<usize>,
    forward: bool,
) -> Option<usize> {
    if image_ids.is_empty() {
        return None;
    }

    let slot = match image_ids.iter().position(|id| Some(*id) == current_slot) {
        Some(position) if forward => image_ids[(position + 1) % image_ids.len()],
        Some(position) => image_ids[(position + image_ids.len() - 1) % image_ids.len()],
        None if forward => image_ids[0],
        None => image_ids[image_ids.len() - 1],
    };

    Some(slot)
}

#[embassy_executor::task]
pub async fn short_press_image_cycle_task() -> ! {
    let mut events = pushbutton::subscribe().expect("button subscriber unavailable in main task");
    let short_press_max = Duration::from_secs(1);

    loop {
        let event = events.next_message_pure().await;
        let (button, held) = match event {
            pushbutton::ButtonEvent::Released { button, held } => (button, held),
            pushbutton::ButtonEvent::Pressed(_) => continue,
        };

        if held >= short_press_max {
            info!(
                "button:navigation ignored button={:?} held={:?} reason=long_press",
                button, held
            );
            continue;
        }

        let forward = match button {
            ButtonId::Button1 => true,
            ButtonId::Button2 => false,
        };

        let image_ids = storage::list_image_ids().await.unwrap_or_default();
        let current_slot = storage::get_active_slot().await;
        let Some(target_slot) = select_cycled_slot(&image_ids, current_slot, forward) else {
            info!("button:navigation ignored reason=no_stored_images");
            continue;
        };

        if storage::set_active_slot(target_slot).await.is_err() {
            warn!(
                "button:navigation failed to persist active slot button={:?} slot={}",
                button, target_slot
            );
        }

        if !led::try_send_led_command(LedCommand::LoadSlot(target_slot)) {
            warn!(
                "button:navigation dropped load slot command button={:?} slot={}",
                button, target_slot
            );
            continue;
        }

        info!(
            "button:navigation button={:?} held={:?} direction={} slot={}",
            button,
            held,
            if forward { "forward" } else { "backward" },
            target_slot
        );
    }
}
