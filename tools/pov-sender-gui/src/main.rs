use std::path::PathBuf;

use iced::{
    Element, Length, Task,
    widget::{button, checkbox, column, container, pick_list, row, scrollable, text, text_input},
};
#[cfg(not(target_arch = "wasm32"))]
use pov_sender_core::list_serial_ports;
use pov_sender_core::{
    AdcDevice, DownloadKind, DownloadRequest, EspNowDelivery, PolarEncodeOptions, SensorOffsets,
    SerialLinkConfig, SpokeCommand, Transport, list_esp_now_peers, request_adc_sample,
    request_storage_stats, send_command, send_download, send_image, send_sensor_offsets,
    send_video_with_max_fps,
};

#[cfg(target_arch = "wasm32")]
mod web_serial;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportUi {
    Ble,
    Espnow,
}

impl TransportUi {
    const ALL: [Self; 2] = [Self::Ble, Self::Espnow];
}

impl std::fmt::Display for TransportUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ble => write!(f, "ble"),
            Self::Espnow => write!(f, "espnow"),
        }
    }
}

impl From<TransportUi> for Transport {
    fn from(value: TransportUi) -> Self {
        match value {
            TransportUi::Ble => Transport::Ble,
            TransportUi::Espnow => Transport::Espnow,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EspNowModeUi {
    Broadcast,
    Stateful,
}

impl EspNowModeUi {
    const ALL: [Self; 2] = [Self::Broadcast, Self::Stateful];
}

impl std::fmt::Display for EspNowModeUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broadcast => write!(f, "broadcast"),
            Self::Stateful => write!(f, "stateful"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EspNowPeerUi {
    mac: [u8; 6],
    label: String,
}

impl EspNowPeerUi {
    fn new(mac: [u8; 6]) -> Self {
        Self {
            mac,
            label: format_mac(mac),
        }
    }
}

impl std::fmt::Display for EspNowPeerUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdcDeviceUi {
    BoardRev,
    HallEffectSensor2,
    BatteryVoltage,
    HallEffectSensor1,
}

impl AdcDeviceUi {
    const ALL: [Self; 4] = [
        Self::BoardRev,
        Self::HallEffectSensor2,
        Self::BatteryVoltage,
        Self::HallEffectSensor1,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::BoardRev => "board-rev",
            Self::HallEffectSensor2 => "hall-effect-sensor-2",
            Self::BatteryVoltage => "battery-voltage",
            Self::HallEffectSensor1 => "hall-effect-sensor-1",
        }
    }
}

impl std::fmt::Display for AdcDeviceUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

impl From<AdcDeviceUi> for AdcDevice {
    fn from(value: AdcDeviceUi) -> Self {
        match value {
            AdcDeviceUi::BoardRev => AdcDevice::BoardRev,
            AdcDeviceUi::HallEffectSensor2 => AdcDevice::HallEffectSensor2,
            AdcDeviceUi::BatteryVoltage => AdcDevice::BatteryVoltage,
            AdcDeviceUi::HallEffectSensor1 => AdcDevice::HallEffectSensor1,
        }
    }
}

fn format_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandTab {
    SendImage,
    SendOta,
    DeviceConfig,
    SetActiveSlot,
    InputLessCommands,
    StorageStats,
    AdcSample,
}

impl CommandTab {
    const ALL: [Self; 7] = [
        Self::SendImage,
        Self::SendOta,
        Self::DeviceConfig,
        Self::SetActiveSlot,
        Self::InputLessCommands,
        Self::StorageStats,
        Self::AdcSample,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::SendImage => "Send Image",
            Self::SendOta => "Send OTA",
            Self::DeviceConfig => "Device Config",
            Self::SetActiveSlot => "Set Active Slot",
            Self::InputLessCommands => "Input-Less Commands",
            Self::StorageStats => "Storage Stats",
            Self::AdcSample => "ADC Sample",
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    RefreshPorts,
    PortsLoaded(Result<Vec<String>, String>),
    #[cfg(target_arch = "wasm32")]
    RequestPort,
    #[cfg(target_arch = "wasm32")]
    PortRequested(Result<Vec<String>, String>),
    RefreshPeers,
    PeersLoaded(Result<Vec<EspNowPeerUi>, String>),
    SelectPort(String),
    SelectTransport(TransportUi),
    SelectEspNowMode(EspNowModeUi),
    SelectPeer(EspNowPeerUi),
    EspNowRetriesChanged(String),
    BaudChanged(String),
    RepeatChanged(String),
    PolarToggled(bool),
    FirstDistanceChanged(String),
    LastDistanceChanged(String),
    GifMaxFpsChanged(String),
    PickImage,
    #[cfg(target_arch = "wasm32")]
    ImagePicked(Result<web_serial::SelectedWebFile, String>),
    PickDownload,
    #[cfg(target_arch = "wasm32")]
    DownloadPicked(Result<web_serial::SelectedWebFile, String>),
    SelectTab(CommandTab),
    SendImage,
    SendOta,
    DisplayOff,
    NextImage,
    RandomizeDisplay,
    ClearAllImages,
    ActiveSlotInputChanged(String),
    SetActiveSlot,
    RequestStorageStats,
    SelectAdcDevice(AdcDeviceUi),
    RequestAdcSample,
    HallOffset0Changed(String),
    HallOffset1Changed(String),
    ImuOffsetChanged(String),
    SetSensorOffsets,
    AdcMonitorSampleRateChanged(String),
    SetAdcMonitorSampleRateHz,
    HybridHallTriggerThresholdChanged(String),
    SetHybridHallTriggerThreshold,
    StorageStatsDone(Result<String, String>),
    AdcSampleDone(Result<String, String>),
    ActionDone(Result<String, String>),
}

struct SenderGui {
    ports: Vec<String>,
    #[cfg(target_arch = "wasm32")]
    web_ports: Vec<web_serial::WebSerialPort>,
    #[cfg(target_arch = "wasm32")]
    web_serial_error_banner: Option<String>,
    selected_port: Option<String>,
    peers: Vec<EspNowPeerUi>,
    selected_peer: Option<EspNowPeerUi>,
    transport: TransportUi,
    esp_now_mode: EspNowModeUi,
    esp_now_retries: String,
    baud: String,
    repeat: String,
    polar_enabled: bool,
    first_led_distance: String,
    last_led_distance: String,
    gif_max_fps: String,
    #[cfg(target_arch = "wasm32")]
    image_file: Option<web_serial::SelectedWebFile>,
    image_path: Option<PathBuf>,
    #[cfg(target_arch = "wasm32")]
    download_file: Option<web_serial::SelectedWebFile>,
    download_path: Option<PathBuf>,
    active_tab: CommandTab,
    hall_offset_0_degrees: String,
    hall_offset_1_degrees: String,
    imu_offset_degrees: String,
    adc_monitor_sample_rate_hz: String,
    hybrid_hall_trigger_threshold: String,
    active_slot_input: String,
    storage_stats_text: String,
    selected_adc_device: AdcDeviceUi,
    adc_sample_text: String,
    status: String,
    busy: bool,
}

impl Default for SenderGui {
    fn default() -> Self {
        Self {
            ports: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            web_ports: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            web_serial_error_banner: None,
            selected_port: None,
            peers: Vec::new(),
            selected_peer: None,
            transport: TransportUi::Espnow,
            esp_now_mode: EspNowModeUi::Broadcast,
            esp_now_retries: "3".to_string(),
            baud: "115200".to_string(),
            repeat: "1".to_string(),
            polar_enabled: false,
            first_led_distance: "18".to_string(),
            last_led_distance: "72".to_string(),
            gif_max_fps: String::new(),
            #[cfg(target_arch = "wasm32")]
            image_file: None,
            image_path: None,
            #[cfg(target_arch = "wasm32")]
            download_file: None,
            download_path: None,
            active_tab: CommandTab::SendImage,
            hall_offset_0_degrees: String::new(),
            hall_offset_1_degrees: String::new(),
            imu_offset_degrees: String::new(),
            adc_monitor_sample_rate_hz: "20".to_string(),
            hybrid_hall_trigger_threshold: "2000".to_string(),
            active_slot_input: String::new(),
            storage_stats_text: "No storage stats requested yet.".to_string(),
            selected_adc_device: AdcDeviceUi::HallEffectSensor1,
            adc_sample_text: "No ADC sample requested yet.".to_string(),
            status: "Ready".to_string(),
            busy: false,
        }
    }
}

fn init_task() -> Task<Message> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Task::perform(
            async { list_serial_ports().map_err(|e| e.to_string()) },
            Message::PortsLoaded,
        )
    }

    #[cfg(target_arch = "wasm32")]
    {
        Task::perform(web_serial::list_port_labels(), Message::PortsLoaded)
    }
}

fn boot() -> (SenderGui, Task<Message>) {
    (SenderGui::default(), init_task())
}

fn update(state: &mut SenderGui, message: Message) -> Task<Message> {
    state.update(message)
}

fn view(state: &SenderGui) -> Element<'_, Message> {
    state.view()
}

impl SenderGui {
    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RefreshPorts => {
                self.status = "Refreshing serial ports...".to_string();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Task::perform(
                        async { list_serial_ports().map_err(|e| e.to_string()) },
                        Message::PortsLoaded,
                    )
                }

                #[cfg(target_arch = "wasm32")]
                {
                    Task::perform(web_serial::list_port_labels(), Message::PortsLoaded)
                }
            }
            Message::PortsLoaded(result) => {
                match result {
                    Ok(ports) => {
                        #[cfg(target_arch = "wasm32")]
                        {
                            self.web_ports = web_serial::cached_ports();
                            self.web_serial_error_banner = None;
                        }

                        if let Some(selected) = &self.selected_port
                            && !ports.iter().any(|p| p == selected)
                        {
                            self.selected_port = None;
                        }
                        self.ports = ports;
                        self.status = format!("Found {} serial port(s)", self.ports.len());
                    }
                    Err(err) => {
                        #[cfg(target_arch = "wasm32")]
                        {
                            self.web_serial_error_banner = Some(err.clone());
                        }
                        self.status = format!("Port refresh failed: {err}");
                    }
                }
                Task::none()
            }
            #[cfg(target_arch = "wasm32")]
            Message::RequestPort => {
                self.status = "Requesting browser serial device permission...".to_string();
                Task::perform(
                    web_serial::request_port_and_list_labels(),
                    Message::PortRequested,
                )
            }
            #[cfg(target_arch = "wasm32")]
            Message::PortRequested(result) => {
                match result {
                    Ok(ports) => {
                        self.web_ports = web_serial::cached_ports();
                        self.web_serial_error_banner = None;
                        self.ports = ports;
                        self.status = format!(
                            "Browser serial device added. {} available.",
                            self.ports.len()
                        );
                    }
                    Err(err) => {
                        self.web_serial_error_banner = Some(err.clone());
                        self.status = format!("Serial permission request failed: {err}");
                    }
                }
                Task::none()
            }
            Message::RefreshPeers => {
                #[cfg(target_arch = "wasm32")]
                {
                    self.status =
                        "ESP-NOW peer refresh is not supported in the web build yet".to_string();
                    return Task::none();
                }

                let port = match self.selected_port.clone() {
                    Some(port) => port,
                    None => {
                        self.status = "Select a serial port first".to_string();
                        return Task::none();
                    }
                };

                let baud = match self.baud.parse::<u32>() {
                    Ok(baud) => baud,
                    Err(_) => {
                        self.status = "Invalid baud rate".to_string();
                        return Task::none();
                    }
                };

                self.status = "Refreshing ESP-NOW peers...".to_string();
                Task::perform(
                    async move {
                        let peers = list_esp_now_peers(&port, baud)
                            .map_err(|e| e.to_string())?
                            .into_iter()
                            .map(|p| EspNowPeerUi::new(p.mac))
                            .collect();
                        Ok(peers)
                    },
                    Message::PeersLoaded,
                )
            }
            Message::PeersLoaded(result) => {
                match result {
                    Ok(peers) => {
                        if let Some(selected) = &self.selected_peer
                            && !peers.iter().any(|p| p == selected)
                        {
                            self.selected_peer = None;
                        }
                        self.peers = peers;
                        self.status = format!("Found {} ESP-NOW peer(s)", self.peers.len());
                    }
                    Err(err) => {
                        self.status = format!("Peer refresh failed: {err}");
                    }
                }
                Task::none()
            }
            Message::SelectPort(port) => {
                self.selected_port = Some(port);
                Task::none()
            }
            Message::SelectTransport(transport) => {
                self.transport = transport;
                Task::none()
            }
            Message::SelectEspNowMode(mode) => {
                self.esp_now_mode = mode;
                Task::none()
            }
            Message::SelectPeer(peer) => {
                self.selected_peer = Some(peer);
                Task::none()
            }
            Message::EspNowRetriesChanged(value) => {
                self.esp_now_retries = value;
                Task::none()
            }
            Message::BaudChanged(value) => {
                self.baud = value;
                Task::none()
            }
            Message::RepeatChanged(value) => {
                self.repeat = value;
                Task::none()
            }
            Message::PolarToggled(enabled) => {
                self.polar_enabled = enabled;
                Task::none()
            }
            Message::FirstDistanceChanged(value) => {
                self.first_led_distance = value;
                Task::none()
            }
            Message::LastDistanceChanged(value) => {
                self.last_led_distance = value;
                Task::none()
            }
            Message::GifMaxFpsChanged(value) => {
                self.gif_max_fps = value;
                Task::none()
            }
            Message::PickImage => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.image_path = rfd::FileDialog::new().pick_file();
                }

                #[cfg(target_arch = "wasm32")]
                {
                    self.status = "Opening browser image picker...".to_string();
                    return Task::perform(
                        web_serial::pick_file("image/*,.gif"),
                        Message::ImagePicked,
                    );
                }
                Task::none()
            }
            #[cfg(target_arch = "wasm32")]
            Message::ImagePicked(result) => {
                match result {
                    Ok(file) => {
                        self.image_file = Some(file);
                        self.status = "Image selected".to_string();
                    }
                    Err(err) => {
                        self.status = format!("Image selection failed: {err}");
                    }
                }
                Task::none()
            }
            Message::PickDownload => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.download_path = rfd::FileDialog::new().pick_file();
                }

                #[cfg(target_arch = "wasm32")]
                {
                    self.status = "Opening browser OTA picker...".to_string();
                    return Task::perform(
                        web_serial::pick_file(".bin,.ota,application/octet-stream"),
                        Message::DownloadPicked,
                    );
                }
                Task::none()
            }
            #[cfg(target_arch = "wasm32")]
            Message::DownloadPicked(result) => {
                match result {
                    Ok(file) => {
                        self.download_file = Some(file);
                        self.status = "OTA payload selected".to_string();
                    }
                    Err(err) => {
                        self.status = format!("OTA selection failed: {err}");
                    }
                }
                Task::none()
            }
            Message::SelectTab(tab) => {
                self.active_tab = tab;
                Task::none()
            }
            Message::SendImage => self.run_send_image(),
            Message::SendOta => self.run_send_ota(),
            Message::DisplayOff => self.run_command(SpokeCommand::DisplayOff),
            Message::NextImage => self.run_command(SpokeCommand::NextImage),
            Message::RandomizeDisplay => self.run_command(SpokeCommand::RandomizeDisplay),
            Message::ClearAllImages => self.run_command(SpokeCommand::ClearAllImages),
            Message::ActiveSlotInputChanged(value) => {
                self.active_slot_input = value;
                Task::none()
            }
            Message::SetActiveSlot => self.run_set_active_slot(),
            Message::RequestStorageStats => self.run_request_storage_stats(),
            Message::SelectAdcDevice(device) => {
                self.selected_adc_device = device;
                Task::none()
            }
            Message::RequestAdcSample => self.run_request_adc_sample(),
            Message::HallOffset0Changed(value) => {
                self.hall_offset_0_degrees = value;
                Task::none()
            }
            Message::HallOffset1Changed(value) => {
                self.hall_offset_1_degrees = value;
                Task::none()
            }
            Message::ImuOffsetChanged(value) => {
                self.imu_offset_degrees = value;
                Task::none()
            }
            Message::SetSensorOffsets => self.run_set_sensor_offsets(),
            Message::AdcMonitorSampleRateChanged(value) => {
                self.adc_monitor_sample_rate_hz = value;
                Task::none()
            }
            Message::SetAdcMonitorSampleRateHz => self.run_set_adc_monitor_sample_rate_hz(),
            Message::HybridHallTriggerThresholdChanged(value) => {
                self.hybrid_hall_trigger_threshold = value;
                Task::none()
            }
            Message::SetHybridHallTriggerThreshold => self.run_set_hybrid_hall_trigger_threshold(),
            Message::StorageStatsDone(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(stats_text) => {
                        self.storage_stats_text = stats_text;
                        "Storage stats received".to_string()
                    }
                    Err(err) => err,
                };
                Task::none()
            }
            Message::AdcSampleDone(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(sample_text) => {
                        self.adc_sample_text = sample_text;
                        "ADC sample received".to_string()
                    }
                    Err(err) => err,
                };
                Task::none()
            }
            Message::ActionDone(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(msg) => msg,
                    Err(err) => err,
                };
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let ports_row = row![
            pick_list(
                self.ports.clone(),
                self.selected_port.clone(),
                Message::SelectPort
            )
            .placeholder("Select serial port")
            .width(Length::Fill),
            button("Refresh").on_press(Message::RefreshPorts),
        ]
        .spacing(10);

        #[cfg(target_arch = "wasm32")]
        {
            let _ = &ports_row;
        }

        #[cfg(target_arch = "wasm32")]
        let ports_row = ports_row.push(button("Connect").on_press(Message::RequestPort));

        let port_panel = column![text("Serial Ports").size(24), ports_row].spacing(10);

        let transport_panel = row![
            text("Transport"),
            pick_list(
                TransportUi::ALL.to_vec(),
                Some(self.transport),
                Message::SelectTransport,
            )
            .width(Length::Shrink),
            text("Baud"),
            text_input("115200", &self.baud).on_input(Message::BaudChanged),
            text("Repeat"),
            text_input("1", &self.repeat).on_input(Message::RepeatChanged),
        ]
        .spacing(10);

        let mut esp_now_panel = row![
            text("ESP-NOW Mode"),
            pick_list(
                EspNowModeUi::ALL.to_vec(),
                Some(self.esp_now_mode),
                Message::SelectEspNowMode,
            )
            .width(Length::Shrink),
            text("Retries"),
            text_input("3", &self.esp_now_retries).on_input(Message::EspNowRetriesChanged),
        ]
        .spacing(10);

        if self.esp_now_mode == EspNowModeUi::Stateful {
            esp_now_panel = esp_now_panel
                .push(button("Refresh Peers").on_press(Message::RefreshPeers))
                .push(
                    pick_list(
                        self.peers.clone(),
                        self.selected_peer.clone(),
                        Message::SelectPeer,
                    )
                    .placeholder("Select target peer")
                    .width(Length::Fill),
                );
        }

        let tabs_row = CommandTab::ALL
            .iter()
            .copied()
            .fold(row![].spacing(10), |row, tab| {
                let label = if tab == self.active_tab {
                    format!("> {}", tab.label())
                } else {
                    tab.label().to_string()
                };
                row.push(button(text(label)).on_press(Message::SelectTab(tab)))
            });

        let actions_panel = column![
            text("Commands").size(24),
            tabs_row,
            self.active_tab_content(),
        ]
        .spacing(10);

        let mut content = column![text("POV Sender").size(30), port_panel, transport_panel]
            .spacing(16)
            .padding(16)
            .width(Length::Fill);

        #[cfg(target_arch = "wasm32")]
        if let Some(message) = &self.web_serial_error_banner {
            let banner = container(text(format!("Web Serial unavailable: {message}")))
                .padding(10)
                .width(Length::Fill);
            content = content.push(banner);
        }

        if self.transport == TransportUi::Espnow {
            content = content.push(esp_now_panel);
        }

        content = content
            .push(scrollable(actions_panel).horizontal().width(Length::Fill))
            .push(text(format!("Status: {}", self.status)));

        container(content).into()
    }
}

impl SenderGui {
    fn active_tab_content(&self) -> Element<'_, Message> {
        match self.active_tab {
            CommandTab::SendImage => self.send_image_tab(),
            CommandTab::SendOta => self.send_ota_tab(),
            CommandTab::DeviceConfig => self.device_config_tab(),
            CommandTab::SetActiveSlot => self.set_active_slot_tab(),
            CommandTab::InputLessCommands => self.input_less_commands_tab(),
            CommandTab::StorageStats => self.storage_stats_tab(),
            CommandTab::AdcSample => self.adc_sample_tab(),
        }
    }

    fn send_image_tab(&self) -> Element<'_, Message> {
        #[cfg(target_arch = "wasm32")]
        let image_path_text = self
            .image_file
            .as_ref()
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "No image selected".to_string());

        #[cfg(not(target_arch = "wasm32"))]
        let image_path_text = self
            .image_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No image selected".to_string());

        let content = column![
            row![
                button("Pick Image").on_press(Message::PickImage),
                button("Send Image").on_press_maybe((!self.busy).then_some(Message::SendImage)),
                text(image_path_text).width(Length::Fill),
            ]
            .spacing(10),
            row![
                checkbox(self.polar_enabled)
                    .label("Polar mode")
                    .on_toggle(Message::PolarToggled),
                text_input("first LED distance", &self.first_led_distance)
                    .on_input(Message::FirstDistanceChanged),
                text_input("last LED distance", &self.last_led_distance)
                    .on_input(Message::LastDistanceChanged),
                text_input("GIF max FPS (optional)", &self.gif_max_fps)
                    .on_input(Message::GifMaxFpsChanged),
            ]
            .spacing(10),
        ]
        .spacing(10);

        container(content).into()
    }

    fn send_ota_tab(&self) -> Element<'_, Message> {
        #[cfg(target_arch = "wasm32")]
        let download_path_text = self
            .download_file
            .as_ref()
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "No payload selected".to_string());

        #[cfg(not(target_arch = "wasm32"))]
        let download_path_text = self
            .download_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No payload selected".to_string());

        let content = row![
            button("Pick OTA").on_press(Message::PickDownload),
            button("Send OTA").on_press_maybe((!self.busy).then_some(Message::SendOta)),
            text(download_path_text).width(Length::Fill),
        ]
        .spacing(10);

        container(content).into()
    }

    fn device_config_tab(&self) -> Element<'_, Message> {
        let offsets = row![
            text_input("hall 0 deg", &self.hall_offset_0_degrees)
                .on_input(Message::HallOffset0Changed),
            text_input("hall 1 deg", &self.hall_offset_1_degrees)
                .on_input(Message::HallOffset1Changed),
            text_input("imu deg", &self.imu_offset_degrees).on_input(Message::ImuOffsetChanged),
            button("Set Offsets").on_press_maybe((!self.busy).then_some(Message::SetSensorOffsets)),
        ]
        .spacing(10);

        let sample_rate = row![
            text_input("sample rate hz", &self.adc_monitor_sample_rate_hz)
                .on_input(Message::AdcMonitorSampleRateChanged),
            button("Set Sample Rate")
                .on_press_maybe((!self.busy).then_some(Message::SetAdcMonitorSampleRateHz)),
        ]
        .spacing(10);

        let hall_threshold = row![
            text_input("hall threshold", &self.hybrid_hall_trigger_threshold)
                .on_input(Message::HybridHallTriggerThresholdChanged),
            button("Set Hall Threshold")
                .on_press_maybe((!self.busy).then_some(Message::SetHybridHallTriggerThreshold),),
        ]
        .spacing(10);

        container(column![offsets, sample_rate, hall_threshold].spacing(10)).into()
    }

    fn input_less_commands_tab(&self) -> Element<'_, Message> {
        let content = row![
            button("Display Off").on_press_maybe((!self.busy).then_some(Message::DisplayOff)),
            button("Next Image").on_press_maybe((!self.busy).then_some(Message::NextImage)),
            button("Randomize Display")
                .on_press_maybe((!self.busy).then_some(Message::RandomizeDisplay)),
            button("Clear All Images")
                .on_press_maybe((!self.busy).then_some(Message::ClearAllImages)),
        ]
        .spacing(10);

        container(content).into()
    }

    fn set_active_slot_tab(&self) -> Element<'_, Message> {
        let content = row![
            text_input("active slot", &self.active_slot_input)
                .on_input(Message::ActiveSlotInputChanged),
            button("Set Active Slot")
                .on_press_maybe((!self.busy).then_some(Message::SetActiveSlot)),
        ]
        .spacing(10);

        container(content).into()
    }

    fn storage_stats_tab(&self) -> Element<'_, Message> {
        let note = "Requires espnow transport + stateful mode + selected peer.";
        let content = column![
            button("Request Storage Stats")
                .on_press_maybe((!self.busy).then_some(Message::RequestStorageStats)),
            text(note),
            text(self.storage_stats_text.clone()),
        ]
        .spacing(10);

        container(content).into()
    }

    fn adc_sample_tab(&self) -> Element<'_, Message> {
        let note = "Requires espnow transport + stateful mode + selected peer.";
        let content = column![
            row![
                text("ADC hookup"),
                pick_list(
                    AdcDeviceUi::ALL.to_vec(),
                    Some(self.selected_adc_device),
                    Message::SelectAdcDevice,
                )
                .width(Length::Shrink),
                button("Request ADC Sample")
                    .on_press_maybe((!self.busy).then_some(Message::RequestAdcSample)),
            ]
            .spacing(10),
            text(note),
            text(self.adc_sample_text.clone()),
        ]
        .spacing(10);

        container(content).into()
    }

    fn parse_link_config(&self) -> Result<SerialLinkConfig, String> {
        let port = self
            .selected_port
            .clone()
            .ok_or_else(|| "Select a serial port first".to_string())?;

        let baud = self
            .baud
            .parse::<u32>()
            .map_err(|_| "Invalid baud rate".to_string())?;

        let repeat = self
            .repeat
            .parse::<usize>()
            .map_err(|_| "Invalid repeat count".to_string())?;

        let esp_now_retries = self
            .esp_now_retries
            .parse::<u8>()
            .map_err(|_| "Invalid ESP-NOW retries".to_string())?;

        let esp_now_delivery = if self.transport == TransportUi::Espnow {
            match self.esp_now_mode {
                EspNowModeUi::Broadcast => EspNowDelivery::Broadcast,
                EspNowModeUi::Stateful => {
                    let peer = self
                        .selected_peer
                        .as_ref()
                        .ok_or_else(|| "Select an ESP-NOW peer for stateful mode".to_string())?;
                    EspNowDelivery::Peer(peer.mac)
                }
            }
        } else {
            EspNowDelivery::Broadcast
        };

        Ok(SerialLinkConfig {
            port,
            baud,
            transport: self.transport.into(),
            esp_now_delivery,
            esp_now_retries,
            repeat,
            inter_packet_delay_ms: 1_000,
        })
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_send_image(&mut self) -> Task<Message> {
        #[cfg(target_arch = "wasm32")]
        {
            let file = match self.image_file.clone() {
                Some(file) => file,
                None => {
                    self.status = "Pick an image file first".to_string();
                    return Task::none();
                }
            };

            let config = match self.parse_link_config() {
                Ok(config) => config,
                Err(err) => {
                    self.status = err;
                    return Task::none();
                }
            };

            let selected_label = config.port.clone();
            let Some(index) = self.ports.iter().position(|p| p == &selected_label) else {
                self.status = "Selected browser serial port is no longer available".to_string();
                return Task::none();
            };

            let Some(port) = self.web_ports.get(index).cloned() else {
                self.status =
                    "Missing browser serial port handle. Reconnect the device.".to_string();
                return Task::none();
            };

            let polar = if self.polar_enabled {
                let first_led_distance = match self.first_led_distance.parse::<f32>() {
                    Ok(value) => value,
                    Err(_) => {
                        self.status = "Invalid first LED distance".to_string();
                        return Task::none();
                    }
                };

                let last_led_distance = match self.last_led_distance.parse::<f32>() {
                    Ok(value) => value,
                    Err(_) => {
                        self.status = "Invalid last LED distance".to_string();
                        return Task::none();
                    }
                };

                Some(PolarEncodeOptions {
                    first_led_distance,
                    last_led_distance,
                })
            } else {
                None
            };

            let gif_max_fps = if self.gif_max_fps.trim().is_empty() {
                None
            } else {
                match self.gif_max_fps.trim().parse::<u16>() {
                    Ok(value) if value > 0 => Some(value),
                    _ => {
                        self.status = "GIF max FPS must be a positive integer".to_string();
                        return Task::none();
                    }
                }
            };

            self.busy = true;
            self.status = if file.name.to_ascii_lowercase().ends_with(".gif") {
                "GIF detected; encoding and sending video...".to_string()
            } else {
                "Sending image...".to_string()
            };

            return Task::perform(
                web_serial::send_image_file_over_web_serial(port, config, file, polar, gif_max_fps),
                Message::ActionDone,
            );
        }

        let image_path = match self.image_path.clone() {
            Some(path) => path,
            None => {
                self.status = "Pick an image file first".to_string();
                return Task::none();
            }
        };

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        let polar = if self.polar_enabled {
            let first_led_distance = match self.first_led_distance.parse::<f32>() {
                Ok(value) => value,
                Err(_) => {
                    self.status = "Invalid first LED distance".to_string();
                    return Task::none();
                }
            };

            let last_led_distance = match self.last_led_distance.parse::<f32>() {
                Ok(value) => value,
                Err(_) => {
                    self.status = "Invalid last LED distance".to_string();
                    return Task::none();
                }
            };

            Some(PolarEncodeOptions {
                first_led_distance,
                last_led_distance,
            })
        } else {
            None
        };

        let is_gif = image_path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"));

        let gif_max_fps = if self.gif_max_fps.trim().is_empty() {
            None
        } else {
            match self.gif_max_fps.trim().parse::<u16>() {
                Ok(value) if value > 0 => Some(value),
                _ => {
                    self.status = "GIF max FPS must be a positive integer".to_string();
                    return Task::none();
                }
            }
        };

        self.busy = true;
        self.status = if is_gif {
            "GIF detected; encoding and sending video...".to_string()
        } else {
            "Sending image...".to_string()
        };

        Task::perform(
            async move {
                let stats = if is_gif {
                    send_video_with_max_fps(&config, &image_path, polar, gif_max_fps)
                        .map_err(|e| e.to_string())?
                } else {
                    send_image(&config, &image_path, polar).map_err(|e| e.to_string())?
                };

                if is_gif {
                    Ok(format!(
                        "Video sent from GIF: {} packet(s), {} transmission(s)",
                        stats.packet_count, stats.total_transmissions
                    ))
                } else {
                    Ok(format!(
                        "Image sent: {} packet(s), {} transmission(s)",
                        stats.packet_count, stats.total_transmissions
                    ))
                }
            },
            Message::ActionDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_send_ota(&mut self) -> Task<Message> {
        #[cfg(target_arch = "wasm32")]
        {
            let file = match self.download_file.clone() {
                Some(file) => file,
                None => {
                    self.status = "Pick an OTA file first".to_string();
                    return Task::none();
                }
            };

            let config = match self.parse_link_config() {
                Ok(config) => config,
                Err(err) => {
                    self.status = err;
                    return Task::none();
                }
            };

            let selected_label = config.port.clone();
            let Some(index) = self.ports.iter().position(|p| p == &selected_label) else {
                self.status = "Selected browser serial port is no longer available".to_string();
                return Task::none();
            };

            let Some(port) = self.web_ports.get(index).cloned() else {
                self.status =
                    "Missing browser serial port handle. Reconnect the device.".to_string();
                return Task::none();
            };

            self.busy = true;
            self.status = "Sending OTA...".to_string();

            return Task::perform(
                web_serial::send_ota_file_over_web_serial(port, config, file),
                Message::ActionDone,
            );
        }

        let file_path = match self.download_path.clone() {
            Some(path) => path,
            None => {
                self.status = "Pick an OTA file first".to_string();
                return Task::none();
            }
        };

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        self.busy = true;
        self.status = "Sending OTA...".to_string();

        Task::perform(
            async move {
                let request = DownloadRequest {
                    file_path: file_path.as_path(),
                    kind: DownloadKind::OtaImage,
                };
                let stats = send_download(&config, request).map_err(|e| e.to_string())?;
                Ok(format!(
                    "OTA sent: {} packet(s), {} transmission(s)",
                    stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_command(&mut self, command: SpokeCommand) -> Task<Message> {
        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        #[cfg(target_arch = "wasm32")]
        {
            let selected_label = config.port.clone();
            let Some(index) = self.ports.iter().position(|p| p == &selected_label) else {
                self.status = "Selected browser serial port is no longer available".to_string();
                return Task::none();
            };

            let Some(port) = self.web_ports.get(index).cloned() else {
                self.status =
                    "Missing browser serial port handle. Reconnect the device.".to_string();
                return Task::none();
            };

            self.busy = true;
            self.status = "Sending command over Web Serial...".to_string();

            return Task::perform(
                web_serial::send_command_over_web_serial(port, config, command),
                Message::ActionDone,
            );
        }

        self.busy = true;
        self.status = "Sending command...".to_string();

        Task::perform(
            async move {
                let stats = send_command(&config, command).map_err(|e| e.to_string())?;
                Ok(format!(
                    "Command sent: {} packet(s), {} transmission(s)",
                    stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_set_sensor_offsets(&mut self) -> Task<Message> {
        let hall_offset_0_degrees = match self.hall_offset_0_degrees.parse::<f32>() {
            Ok(value) => value,
            Err(_) => {
                self.status = "Invalid hall offset 0".to_string();
                return Task::none();
            }
        };

        let hall_offset_1_degrees = match self.hall_offset_1_degrees.parse::<f32>() {
            Ok(value) => value,
            Err(_) => {
                self.status = "Invalid hall offset 1".to_string();
                return Task::none();
            }
        };

        let imu_offset_degrees = match self.imu_offset_degrees.parse::<f32>() {
            Ok(value) => value,
            Err(_) => {
                self.status = "Invalid IMU offset".to_string();
                return Task::none();
            }
        };

        #[cfg(target_arch = "wasm32")]
        {
            return self.run_command(SpokeCommand::SetSensorOffsets {
                hall_offset_0_degrees,
                hall_offset_1_degrees,
                imu_offset_degrees,
            });
        }

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        self.busy = true;
        self.status = "Persisting sensor offsets...".to_string();

        Task::perform(
            async move {
                let stats = send_sensor_offsets(
                    &config,
                    SensorOffsets {
                        hall_offset_0_degrees,
                        hall_offset_1_degrees,
                        imu_offset_degrees,
                    },
                )
                .map_err(|e| e.to_string())?;
                Ok(format!(
                    "Sensor offsets sent: {} packet(s), {} transmission(s). Reboot firmware to apply.",
                    stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_set_adc_monitor_sample_rate_hz(&mut self) -> Task<Message> {
        let hz = match self.adc_monitor_sample_rate_hz.parse::<u16>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.status = "Invalid ADC monitor sample rate".to_string();
                return Task::none();
            }
        };

        self.run_command_with_status(
            SpokeCommand::SetAdcMonitorSampleRateHz { hz },
            "Persisting ADC monitor sample rate...",
            format!("ADC monitor sample rate sent: reboot firmware to apply (hz={hz})."),
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_set_hybrid_hall_trigger_threshold(&mut self) -> Task<Message> {
        let threshold = match self.hybrid_hall_trigger_threshold.parse::<u16>() {
            Ok(value) if value > 0 => value,
            _ => {
                self.status = "Invalid hall trigger threshold".to_string();
                return Task::none();
            }
        };

        self.run_command_with_status(
            SpokeCommand::SetHybridHallTriggerThreshold { threshold },
            "Persisting hall trigger threshold...",
            format!(
                "Hall trigger threshold sent: reboot firmware to apply (threshold={threshold})."
            ),
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_command_with_status(
        &mut self,
        command: SpokeCommand,
        pending_status: &str,
        success_status: String,
    ) -> Task<Message> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = &success_status;
            self.busy = true;
            self.status = pending_status.to_string();
            return self.run_command(command);
        }

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        self.busy = true;
        self.status = pending_status.to_string();

        Task::perform(
            async move {
                let stats = send_command(&config, command).map_err(|e| e.to_string())?;
                Ok(format!(
                    "{} {} packet(s), {} transmission(s)",
                    success_status, stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_set_active_slot(&mut self) -> Task<Message> {
        let slot = match self.active_slot_input.trim().parse::<u32>() {
            Ok(slot) => slot,
            Err(_) => {
                self.status = "Invalid active slot".to_string();
                return Task::none();
            }
        };

        #[cfg(target_arch = "wasm32")]
        {
            return self.run_command(SpokeCommand::SetActiveSlot { slot });
        }

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        self.busy = true;
        self.status = format!("Setting active slot to {}...", slot);

        Task::perform(
            async move {
                let stats = send_command(&config, SpokeCommand::SetActiveSlot { slot })
                    .map_err(|e| e.to_string())?;
                Ok(format!(
                    "SetActiveSlot sent: {} packet(s), {} transmission(s)",
                    stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_request_storage_stats(&mut self) -> Task<Message> {
        #[cfg(target_arch = "wasm32")]
        {
            self.status =
                "Storage stats is not available in the web build yet (response reads pending)"
                    .to_string();
            return Task::none();
        }

        if self.transport != TransportUi::Espnow {
            self.status = "Storage stats requires espnow transport".to_string();
            return Task::none();
        }

        if self.esp_now_mode != EspNowModeUi::Stateful {
            self.status = "Storage stats requires stateful espnow mode".to_string();
            return Task::none();
        }

        if self.selected_peer.is_none() {
            self.status = "Select a target peer first".to_string();
            return Task::none();
        }

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        self.busy = true;
        self.status = "Requesting storage stats...".to_string();

        Task::perform(
            async move {
                let stats = request_storage_stats(&config).map_err(|e| e.to_string())?;
                Ok(format!(
                    "total_bytes={}\nused_bytes={}\nfree_bytes={}\nimage_count={}\nactive_image_id={}",
                    stats.total_bytes,
                    stats.used_bytes,
                    stats.free_bytes,
                    stats.image_count,
                    stats
                        .active_image_id
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "none".to_string())
                ))
            },
            Message::StorageStatsDone,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
    fn run_request_adc_sample(&mut self) -> Task<Message> {
        #[cfg(target_arch = "wasm32")]
        {
            self.status =
                "ADC sample request is not available in the web build yet (response reads pending)"
                    .to_string();
            return Task::none();
        }

        if self.transport != TransportUi::Espnow {
            self.status = "ADC sample request requires espnow transport".to_string();
            return Task::none();
        }

        if self.esp_now_mode != EspNowModeUi::Stateful {
            self.status = "ADC sample request requires stateful espnow mode".to_string();
            return Task::none();
        }

        if self.selected_peer.is_none() {
            self.status = "Select a target peer first".to_string();
            return Task::none();
        }

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Task::none();
            }
        };

        let device = self.selected_adc_device;
        self.busy = true;
        self.status = format!("Requesting ADC sample from {}...", device.label());

        Task::perform(
            async move {
                let sample =
                    request_adc_sample(&config, device.into()).map_err(|e| e.to_string())?;
                Ok(format!("device={}\nraw={}", device.label(), sample.raw))
            },
            Message::AdcSampleDone,
        )
    }
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        .title("POV Sender GUI")
        .run()
}
