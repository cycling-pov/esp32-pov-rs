use std::path::PathBuf;

use iced::{
    Application, Command, Element, Length, Settings, Theme,
    widget::{button, checkbox, column, container, pick_list, row, text, text_input},
};
use pov_sender_core::{
    DownloadKind, DownloadRequest, PolarEncodeOptions, SerialLinkConfig, SpokeCommand, Transport,
    list_serial_ports, send_command, send_download, send_image,
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
enum DownloadKindUi {
    DisplayImage,
    OtaImage,
    Video,
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
    SelectPort(String),
    SelectTransport(TransportUi),
    BaudChanged(String),
    RepeatChanged(String),
    PolarToggled(bool),
    FirstDistanceChanged(String),
    LastDistanceChanged(String),
    PickImage,
    PickDownload,
    SelectDownloadKind(DownloadKindUi),
    SendImage,
    SendDownload,
    DisplayOff,
    NextImage,
    RandomizeDisplay,
    ActionDone(Result<String, String>),
}

struct SenderGui {
    ports: Vec<String>,
    selected_port: Option<String>,
    transport: TransportUi,
    baud: String,
    repeat: String,
    polar_enabled: bool,
    first_led_distance: String,
    last_led_distance: String,
    image_path: Option<PathBuf>,
    download_path: Option<PathBuf>,
    download_kind: DownloadKindUi,
    status: String,
    busy: bool,
}

impl Default for SenderGui {
    fn default() -> Self {
        Self {
            ports: Vec::new(),
            selected_port: None,
            transport: TransportUi::Espnow,
            baud: "115200".to_string(),
            repeat: "1".to_string(),
            polar_enabled: false,
            first_led_distance: "18".to_string(),
            last_led_distance: "72".to_string(),
            image_path: None,
            download_path: None,
            download_kind: DownloadKindUi::DisplayImage,
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
            Message::SelectPort(port) => {
                self.selected_port = Some(port);
                Command::none()
            }
            Message::SelectTransport(transport) => {
                self.transport = transport;
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
            Message::SendImage => self.run_send_image(),
            Message::SendDownload => self.run_send_download(),
            Message::DisplayOff => self.run_command(SpokeCommand::DisplayOff),
            Message::NextImage => self.run_command(SpokeCommand::NextImage),
            Message::RandomizeDisplay => self.run_command(SpokeCommand::RandomizeDisplay),
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

        let image_path_text = self
            .image_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No image selected".to_string());

        let download_path_text = self
            .download_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No payload selected".to_string());

        let polar_row = row![
            checkbox("Polar mode", self.polar_enabled).on_toggle(Message::PolarToggled),
            text_input("first LED distance", &self.first_led_distance)
                .on_input(Message::FirstDistanceChanged),
            text_input("last LED distance", &self.last_led_distance)
                .on_input(Message::LastDistanceChanged),
        ]
        .spacing(10);

        let actions_panel = column![
            text("Actions").size(24),
            row![
                button("Pick Image").on_press(Message::PickImage),
                button("Send Image").on_press_maybe((!self.busy).then_some(Message::SendImage)),
                text(image_path_text).width(Length::Fill),
            ]
            .spacing(10),
            polar_row,
            row![
                pick_list(
                    DownloadKindUi::ALL.to_vec(),
                    Some(self.download_kind),
                    Message::SelectDownloadKind,
                )
                .width(Length::Shrink),
                button("Pick Download").on_press(Message::PickDownload),
                button("Send Download")
                    .on_press_maybe((!self.busy).then_some(Message::SendDownload)),
                text(download_path_text).width(Length::Fill),
            ]
            .spacing(10),
            row![
                button("Display Off").on_press_maybe((!self.busy).then_some(Message::DisplayOff)),
                button("Next Image").on_press_maybe((!self.busy).then_some(Message::NextImage)),
                button("Randomize Display")
                    .on_press_maybe((!self.busy).then_some(Message::RandomizeDisplay)),
            ]
            .spacing(10),
        ]
        .spacing(10);

        let content = column![
            text("POV Sender").size(30),
            port_panel,
            transport_panel,
            actions_panel,
            text(format!("Status: {}", self.status)),
        ]
        .spacing(16)
        .padding(16)
        .width(Length::Fill);

        container(content).into()
    }
}

impl SenderGui {
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

        Ok(SerialLinkConfig {
            port,
            baud,
            transport: self.transport.into(),
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
}

fn main() -> iced::Result {
    SenderGui::run(Settings::default())
}
