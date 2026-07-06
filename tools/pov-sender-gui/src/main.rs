use std::path::PathBuf;

use iced::{
    Application, Command, Element, Length, Settings, Theme,
    widget::{button, checkbox, column, container, pick_list, row, text, text_input},
};
use pov_sender_core::{
    DownloadKind, DownloadRequest, EspNowDelivery, PolarEncodeOptions, SensorOffsets,
    SerialLinkConfig, SpokeCommand, Transport, list_esp_now_peers, list_serial_ports, send_command,
    send_download, send_image, send_sensor_offsets,
};

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

fn format_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DownloadKindUi {
    DisplayImage,
    OtaImage,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandTab {
    SendImage,
    SendDownload,
    SetOffsets,
    InputLessCommands,
}

impl CommandTab {
    const ALL: [Self; 4] = [
        Self::SendImage,
        Self::SendDownload,
        Self::SetOffsets,
        Self::InputLessCommands,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::SendImage => "Send Image",
            Self::SendDownload => "Send Download",
            Self::SetOffsets => "Set Offsets",
            Self::InputLessCommands => "Input-Less Commands",
        }
    }
}

impl DownloadKindUi {
    const ALL: [Self; 3] = [Self::DisplayImage, Self::OtaImage, Self::Video];
}

impl std::fmt::Display for DownloadKindUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DisplayImage => write!(f, "display-image"),
            Self::OtaImage => write!(f, "ota-image"),
            Self::Video => write!(f, "video"),
        }
    }
}

impl From<DownloadKindUi> for DownloadKind {
    fn from(value: DownloadKindUi) -> Self {
        match value {
            DownloadKindUi::DisplayImage => DownloadKind::DisplayImage,
            DownloadKindUi::OtaImage => DownloadKind::OtaImage,
            DownloadKindUi::Video => DownloadKind::Video,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    RefreshPorts,
    PortsLoaded(Result<Vec<String>, String>),
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
    PickImage,
    PickDownload,
    SelectDownloadKind(DownloadKindUi),
    SelectTab(CommandTab),
    SendImage,
    SendDownload,
    DisplayOff,
    NextImage,
    RandomizeDisplay,
    HallOffset0Changed(String),
    HallOffset1Changed(String),
    ImuOffsetChanged(String),
    SetSensorOffsets,
    ActionDone(Result<String, String>),
}

struct SenderGui {
    ports: Vec<String>,
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
    image_path: Option<PathBuf>,
    download_path: Option<PathBuf>,
    download_kind: DownloadKindUi,
    active_tab: CommandTab,
    hall_offset_0_degrees: String,
    hall_offset_1_degrees: String,
    imu_offset_degrees: String,
    status: String,
    busy: bool,
}

impl Default for SenderGui {
    fn default() -> Self {
        Self {
            ports: Vec::new(),
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
            image_path: None,
            download_path: None,
            download_kind: DownloadKindUi::DisplayImage,
            active_tab: CommandTab::SendImage,
            hall_offset_0_degrees: String::new(),
            hall_offset_1_degrees: String::new(),
            imu_offset_degrees: String::new(),
            status: "Ready".to_string(),
            busy: false,
        }
    }
}

impl Application for SenderGui {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        (
            Self::default(),
            Command::perform(
                async { list_serial_ports().map_err(|e| e.to_string()) },
                Message::PortsLoaded,
            ),
        )
    }

    fn title(&self) -> String {
        "POV Sender GUI".to_string()
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::RefreshPorts => {
                self.status = "Refreshing serial ports...".to_string();
                Command::perform(
                    async { list_serial_ports().map_err(|e| e.to_string()) },
                    Message::PortsLoaded,
                )
            }
            Message::PortsLoaded(result) => {
                match result {
                    Ok(ports) => {
                        if let Some(selected) = &self.selected_port
                            && !ports.iter().any(|p| p == selected)
                        {
                            self.selected_port = None;
                        }
                        self.ports = ports;
                        self.status = format!("Found {} serial port(s)", self.ports.len());
                    }
                    Err(err) => {
                        self.status = format!("Port refresh failed: {err}");
                    }
                }
                Command::none()
            }
            Message::RefreshPeers => {
                let port = match self.selected_port.clone() {
                    Some(port) => port,
                    None => {
                        self.status = "Select a serial port first".to_string();
                        return Command::none();
                    }
                };

                let baud = match self.baud.parse::<u32>() {
                    Ok(baud) => baud,
                    Err(_) => {
                        self.status = "Invalid baud rate".to_string();
                        return Command::none();
                    }
                };

                self.status = "Refreshing ESP-NOW peers...".to_string();
                Command::perform(
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
                Command::none()
            }
            Message::SelectPort(port) => {
                self.selected_port = Some(port);
                Command::none()
            }
            Message::SelectTransport(transport) => {
                self.transport = transport;
                Command::none()
            }
            Message::SelectEspNowMode(mode) => {
                self.esp_now_mode = mode;
                Command::none()
            }
            Message::SelectPeer(peer) => {
                self.selected_peer = Some(peer);
                Command::none()
            }
            Message::EspNowRetriesChanged(value) => {
                self.esp_now_retries = value;
                Command::none()
            }
            Message::BaudChanged(value) => {
                self.baud = value;
                Command::none()
            }
            Message::RepeatChanged(value) => {
                self.repeat = value;
                Command::none()
            }
            Message::PolarToggled(enabled) => {
                self.polar_enabled = enabled;
                Command::none()
            }
            Message::FirstDistanceChanged(value) => {
                self.first_led_distance = value;
                Command::none()
            }
            Message::LastDistanceChanged(value) => {
                self.last_led_distance = value;
                Command::none()
            }
            Message::PickImage => {
                self.image_path = rfd::FileDialog::new().pick_file();
                Command::none()
            }
            Message::PickDownload => {
                self.download_path = rfd::FileDialog::new().pick_file();
                Command::none()
            }
            Message::SelectDownloadKind(kind) => {
                self.download_kind = kind;
                Command::none()
            }
            Message::SelectTab(tab) => {
                self.active_tab = tab;
                Command::none()
            }
            Message::SendImage => self.run_send_image(),
            Message::SendDownload => self.run_send_download(),
            Message::DisplayOff => self.run_command(SpokeCommand::DisplayOff),
            Message::NextImage => self.run_command(SpokeCommand::NextImage),
            Message::RandomizeDisplay => self.run_command(SpokeCommand::RandomizeDisplay),
            Message::HallOffset0Changed(value) => {
                self.hall_offset_0_degrees = value;
                Command::none()
            }
            Message::HallOffset1Changed(value) => {
                self.hall_offset_1_degrees = value;
                Command::none()
            }
            Message::ImuOffsetChanged(value) => {
                self.imu_offset_degrees = value;
                Command::none()
            }
            Message::SetSensorOffsets => self.run_set_sensor_offsets(),
            Message::ActionDone(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(msg) => msg,
                    Err(err) => err,
                };
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let port_panel = column![
            text("Serial Ports").size(24),
            row![
                pick_list(
                    self.ports.clone(),
                    self.selected_port.clone(),
                    Message::SelectPort
                )
                .placeholder("Select serial port")
                .width(Length::Fill),
                button("Refresh").on_press(Message::RefreshPorts),
            ]
            .spacing(10),
        ]
        .spacing(10);

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

        if self.transport == TransportUi::Espnow {
            content = content.push(esp_now_panel);
        }

        content = content
            .push(actions_panel)
            .push(text(format!("Status: {}", self.status)));

        container(content).into()
    }
}

impl SenderGui {
    fn active_tab_content(&self) -> Element<'_, Message> {
        match self.active_tab {
            CommandTab::SendImage => self.send_image_tab(),
            CommandTab::SendDownload => self.send_download_tab(),
            CommandTab::SetOffsets => self.set_offsets_tab(),
            CommandTab::InputLessCommands => self.input_less_commands_tab(),
        }
    }

    fn send_image_tab(&self) -> Element<'_, Message> {
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
                checkbox("Polar mode", self.polar_enabled).on_toggle(Message::PolarToggled),
                text_input("first LED distance", &self.first_led_distance)
                    .on_input(Message::FirstDistanceChanged),
                text_input("last LED distance", &self.last_led_distance)
                    .on_input(Message::LastDistanceChanged),
            ]
            .spacing(10),
        ]
        .spacing(10);

        container(content).into()
    }

    fn send_download_tab(&self) -> Element<'_, Message> {
        let download_path_text = self
            .download_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No payload selected".to_string());

        let content = row![
            pick_list(
                DownloadKindUi::ALL.to_vec(),
                Some(self.download_kind),
                Message::SelectDownloadKind,
            )
            .width(Length::Shrink),
            button("Pick Download").on_press(Message::PickDownload),
            button("Send Download").on_press_maybe((!self.busy).then_some(Message::SendDownload)),
            text(download_path_text).width(Length::Fill),
        ]
        .spacing(10);

        container(content).into()
    }

    fn set_offsets_tab(&self) -> Element<'_, Message> {
        let content = row![
            text_input("hall 0 deg", &self.hall_offset_0_degrees)
                .on_input(Message::HallOffset0Changed),
            text_input("hall 1 deg", &self.hall_offset_1_degrees)
                .on_input(Message::HallOffset1Changed),
            text_input("imu deg", &self.imu_offset_degrees).on_input(Message::ImuOffsetChanged),
            button("Set Offsets").on_press_maybe((!self.busy).then_some(Message::SetSensorOffsets)),
        ]
        .spacing(10);

        container(content).into()
    }

    fn input_less_commands_tab(&self) -> Element<'_, Message> {
        let content = row![
            button("Display Off").on_press_maybe((!self.busy).then_some(Message::DisplayOff)),
            button("Next Image").on_press_maybe((!self.busy).then_some(Message::NextImage)),
            button("Randomize Display")
                .on_press_maybe((!self.busy).then_some(Message::RandomizeDisplay)),
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

    fn run_send_image(&mut self) -> Command<Message> {
        let image_path = match self.image_path.clone() {
            Some(path) => path,
            None => {
                self.status = "Pick an image file first".to_string();
                return Command::none();
            }
        };

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Command::none();
            }
        };

        let polar = if self.polar_enabled {
            let first_led_distance = match self.first_led_distance.parse::<f32>() {
                Ok(value) => value,
                Err(_) => {
                    self.status = "Invalid first LED distance".to_string();
                    return Command::none();
                }
            };

            let last_led_distance = match self.last_led_distance.parse::<f32>() {
                Ok(value) => value,
                Err(_) => {
                    self.status = "Invalid last LED distance".to_string();
                    return Command::none();
                }
            };

            Some(PolarEncodeOptions {
                first_led_distance,
                last_led_distance,
            })
        } else {
            None
        };

        self.busy = true;
        self.status = "Sending image...".to_string();

        Command::perform(
            async move {
                let stats = send_image(&config, &image_path, polar).map_err(|e| e.to_string())?;
                Ok(format!(
                    "Image sent: {} packet(s), {} transmission(s)",
                    stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    fn run_send_download(&mut self) -> Command<Message> {
        let file_path = match self.download_path.clone() {
            Some(path) => path,
            None => {
                self.status = "Pick a download file first".to_string();
                return Command::none();
            }
        };

        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Command::none();
            }
        };

        let kind = DownloadKind::from(self.download_kind);

        self.busy = true;
        self.status = "Sending download...".to_string();

        Command::perform(
            async move {
                let request = DownloadRequest {
                    file_path: file_path.as_path(),
                    kind,
                };
                let stats = send_download(&config, request).map_err(|e| e.to_string())?;
                Ok(format!(
                    "Download sent: {} packet(s), {} transmission(s)",
                    stats.packet_count, stats.total_transmissions
                ))
            },
            Message::ActionDone,
        )
    }

    fn run_command(&mut self, command: SpokeCommand) -> Command<Message> {
        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Command::none();
            }
        };

        self.busy = true;
        self.status = "Sending command...".to_string();

        Command::perform(
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

    fn run_set_sensor_offsets(&mut self) -> Command<Message> {
        let config = match self.parse_link_config() {
            Ok(config) => config,
            Err(err) => {
                self.status = err;
                return Command::none();
            }
        };

        let hall_offset_0_degrees = match self.hall_offset_0_degrees.parse::<f32>() {
            Ok(value) => value,
            Err(_) => {
                self.status = "Invalid hall offset 0".to_string();
                return Command::none();
            }
        };

        let hall_offset_1_degrees = match self.hall_offset_1_degrees.parse::<f32>() {
            Ok(value) => value,
            Err(_) => {
                self.status = "Invalid hall offset 1".to_string();
                return Command::none();
            }
        };

        let imu_offset_degrees = match self.imu_offset_degrees.parse::<f32>() {
            Ok(value) => value,
            Err(_) => {
                self.status = "Invalid IMU offset".to_string();
                return Command::none();
            }
        };

        self.busy = true;
        self.status = "Persisting sensor offsets...".to_string();

        Command::perform(
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
}

fn main() -> iced::Result {
    SenderGui::run(Settings::default())
}
