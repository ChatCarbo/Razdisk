#![windows_subsystem = "windows"]

use eframe::egui;
use std::ffi::CStr;
use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives};
use windows::Win32::System::Ioctl::{
    DRIVE_LAYOUT_INFORMATION_EX, PARTITION_STYLE_GPT, PARTITION_STYLE_MBR,
};
use windows::Win32::System::IO::DeviceIoControl;

// ---------------------------------------------------------------------------
// Constantes Windows
// ---------------------------------------------------------------------------
const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D1080;
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
const IOCTL_DISK_GET_DRIVE_LAYOUT_EX: u32 = 0x00070050;
const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007405C;

const STORAGE_DEVICE_PROPERTY: u32 = 0;
const CREATE_NO_WINDOW: u32 = 0x08000000;

// ---------------------------------------------------------------------------
// Structures Windows
// ---------------------------------------------------------------------------
#[repr(C)]
struct StorageDeviceNumber {
    device_type: u32,
    device_number: u32,
    partition_number: u32,
}

#[repr(C)]
struct StoragePropertyQuery {
    property_id: u32,
    query_type: u32,
    additional_parameters: [u8; 4],
}

#[repr(C)]
struct StorageDeviceDescriptor {
    version: u32,
    size: u32,
    device_type: u8,
    device_type_modifier: u8,
    removable_media: u8,
    command_queueing: u8,
    vendor_id_offset: u32,
    product_id_offset: u32,
    product_revision_offset: u32,
    serial_number_offset: u32,
    bus_type: u32,
    raw_properties_length: u32,
}

#[repr(C)]
struct GetLengthInformation {
    length: i64,
}

// ---------------------------------------------------------------------------
// Point d'entrée
// ---------------------------------------------------------------------------
fn main() -> Result<(), eframe::Error> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::IsUserAnAdmin;
        if !unsafe { IsUserAnAdmin().as_bool() } {
            rfd::MessageDialog::new()
                .set_title("RazDisk — Privilèges requis")
                .set_description("RazDisk nécessite des droits administrateur.")
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Ok(());
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([490.0, 720.0])
            .with_min_inner_size([440.0, 650.0])
            .with_title("Formater avec RazDisk"),
        ..Default::default()
    };

    eframe::run_native(
        "RazDisk",
        options,
        Box::new(|cc| {
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::light();
            style.spacing.item_spacing = egui::vec2(8.0, 10.0);
            style.spacing.button_padding = egui::vec2(12.0, 6.0);
            cc.egui_ctx.set_style(style);
            Ok(Box::new(RazDiskApp::new()))
        }),
    )
}

// ---------------------------------------------------------------------------
// Types & Choix
// ---------------------------------------------------------------------------
#[derive(PartialEq, Clone, Copy)]
enum FileSystemChoice {
    Fat32,
    ExFat,
    Ntfs,
}

impl FileSystemChoice {
    fn display(&self) -> &'static str {
        match self {
            Self::Fat32 => "FAT32",
            Self::ExFat => "exFAT",
            Self::Ntfs => "NTFS",
        }
    }
    fn diskpart_arg(&self) -> &'static str {
        match self {
            Self::Fat32 => "FAT32",
            Self::ExFat => "EXFAT",
            Self::Ntfs => "NTFS",
        }
    }
    fn max_label_len(&self) -> usize {
        match self {
            Self::Ntfs => 32,
            Self::Fat32 | Self::ExFat => 11,
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum PartitionSchemeChoice {
    Keep,
    Mbr,
    Gpt,
}

impl PartitionSchemeChoice {
    fn display(&self) -> &'static str {
        match self {
            Self::Keep => "Conserver (Actuel)",
            Self::Mbr => "MBR (BIOS / Compatibilité)",
            Self::Gpt => "GPT (UEFI / > 2 To)",
        }
    }
    fn diskpart_cmd(&self) -> Option<&'static str> {
        match self {
            Self::Keep => None,
            Self::Mbr => Some("convert mbr"),
            Self::Gpt => Some("convert gpt"),
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum ClusterSizeChoice {
    Default,
    Kb4,
    Kb8,
    Kb16,
    Kb32,
    Kb64,
    Kb128,
}

impl ClusterSizeChoice {
    fn display(&self) -> &'static str {
        match self {
            Self::Default => "Par défaut",
            Self::Kb4 => "4 Ko (4096 octets)",
            Self::Kb8 => "8 Ko",
            Self::Kb16 => "16 Ko",
            Self::Kb32 => "32 Ko",
            Self::Kb64 => "64 Ko",
            Self::Kb128 => "128 Ko",
        }
    }
    fn diskpart_unit(&self) -> Option<u32> {
        match self {
            Self::Default => None,
            Self::Kb4 => Some(4096),
            Self::Kb8 => Some(8192),
            Self::Kb16 => Some(16384),
            Self::Kb32 => Some(32768),
            Self::Kb64 => Some(65536),
            Self::Kb128 => Some(131072),
        }
    }
}

#[derive(Clone)]
#[allow(dead_code)]
struct DebugDriveInfo {
    physical_number: u32,
    bus_type: Option<u32>,
    bus_label: String,
    model_name: String,
    letter: Option<char>,
    size_bytes: u64,
    accepted: bool,
    reason: String,
}

#[derive(Clone)]
struct RemovableDrive {
    letter: Option<char>,
    physical_drive: u32,
    model_name: String,
    size_bytes: u64,
    free_bytes: Option<u64>,
    bus_type: u32,
}

impl RemovableDrive {
    fn label(&self) -> String {
        let letter_str = self
            .letter
            .map(|c| format!("{}:", c))
            .unwrap_or_else(|| "Sans lettre".into());
        let size_gb = self.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        format!(
            "{} — {} ({:.1} Go) [{}]",
            letter_str,
            self.model_name,
            size_gb,
            bus_label_of(self.bus_type)
        )
    }
}

fn bus_label_of(bus: u32) -> &'static str {
    match bus {
        0x00 => "SCSI", 0x01 => "ATAPI", 0x02 => "ATA", 0x03 => "1394",
        0x04 => "FireWire", 0x05 => "SSA", 0x06 => "Fibre", 0x07 => "USB",
        0x08 => "RAID", 0x09 => "iSCSI", 0x0A => "SAS", 0x0B => "SATA",
        0x0C => "SD", 0x0D => "MMC", 0x0E => "Virtual", 0x0F => "VHD",
        0x10 => "Spaces", 0x11 => "NVMe", 0x12 => "SCM", 0x13 => "UFS",
        _ => "Inconnu",
    }
}

#[derive(Clone)]
#[allow(dead_code)]
struct PartitionInfo {
    partition_number: u32,
    start_lba: u64,
    sectors: u64,
    style: String,
}

impl PartitionInfo {
    fn size_mb(&self) -> f64 {
        self.sectors as f64 * 512.0 / (1024.0 * 1024.0)
    }
}

enum Status {
    None,
    Info(String),
    Success(String),
    Error(String),
}

enum FormatMsg {
    Progress(f32, String),
    Done(Result<(), String>),
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------
struct RazDiskApp {
    all_drives: Vec<DebugDriveInfo>,
    drives: Vec<RemovableDrive>,
    selected_drive: Option<usize>,
    partitions: Vec<PartitionInfo>,
    selected_to_delete: Vec<bool>,
    target_fs: FileSystemChoice,
    partition_scheme: PartitionSchemeChoice,
    cluster_size: ClusterSizeChoice,
    partition_label: String,
    safety_confirmed: bool,
    status: Status,
    is_formatting: bool,
    progress_val: f32,
    progress_step: String,
    rx_msg: Option<Receiver<FormatMsg>>,
}

impl RazDiskApp {
    fn new() -> Self {
        let mut app = Self {
            all_drives: Vec::new(),
            drives: Vec::new(),
            selected_drive: None,
            partitions: Vec::new(),
            selected_to_delete: Vec::new(),
            target_fs: FileSystemChoice::ExFat,
            partition_scheme: PartitionSchemeChoice::Keep,
            cluster_size: ClusterSizeChoice::Default,
            partition_label: "RAZDISK".to_string(),
            safety_confirmed: false,
            status: Status::None,
            is_formatting: false,
            progress_val: 0.0,
            progress_step: String::new(),
            rx_msg: None,
        };
        app.refresh_drives();
        app
    }

    fn refresh_drives(&mut self) {
        let (all, accepted) = enumerate_all_drives();
        self.all_drives = all;
        self.drives = accepted;
        self.selected_drive = None;
        self.partitions.clear();
        self.selected_to_delete.clear();
        self.safety_confirmed = false;
        self.status = Status::Info(format!(
            "{} disque(s) externe(s) détecté(s).",
            self.drives.len()
        ));
    }

    fn load_partitions(&mut self) {
        let Some(idx) = self.selected_drive else { return };
        let drive = &self.drives[idx];

        self.partitions.clear();
        self.selected_to_delete.clear();
        self.safety_confirmed = false;

        match get_partitions(drive.physical_drive) {
            Ok(parts) => {
                for p in parts {
                    self.partitions.push(p);
                    self.selected_to_delete.push(false);
                }
                self.status = Status::Info(format!(
                    "{} partition(s) trouvée(s).",
                    self.partitions.len()
                ));
            }
            Err(e) => {
                self.status = Status::Error(format!("Erreur lecture : {}", e));
            }
        }
    }

    fn execute(&mut self) {
        let Some(idx) = self.selected_drive else { return };
        let drive = &self.drives[idx];
        let disk_number = drive.physical_drive;
        let drive_label = drive.label();

        let to_delete: Vec<usize> = self
            .selected_to_delete
            .iter()
            .enumerate()
            .filter_map(|(i, &sel)| if sel { Some(i) } else { None })
            .collect();

        if to_delete.is_empty() {
            self.status = Status::Error("Cochez au moins une partition à supprimer.".into());
            return;
        }

        let all_selected = self.selected_to_delete.iter().all(|&b| b);
        let active_scheme = if all_selected {
            self.partition_scheme
        } else {
            PartitionSchemeChoice::Keep
        };

        let clean_label = sanitize_label(&self.partition_label, self.target_fs.max_label_len());

        let msg = format!(
            "⚠️  DERNIÈRE CONFIRMATION\n\n\
             • Cible : {}\n\
             • Action : Supprimer {} partition(s)\n\
             • Schéma : {}\n\
             • Format : {} ({})\n\
             • Nom : '{}'\n\n\
             TOUTES LES DONNÉES seront DÉFINITIVEMENT PERDUES.\n\n\
             Lancer le formatage ?",
            drive_label,
            to_delete.len(),
            active_scheme.display(),
            self.target_fs.display(),
            self.cluster_size.display(),
            clean_label
        );

        let confirmed = rfd::MessageDialog::new()
            .set_title("Confirmer le formatage")
            .set_description(&msg)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show();

        if confirmed != rfd::MessageDialogResult::Yes {
            self.status = Status::Info("Opération annulée.".into());
            return;
        }

        let partition_numbers: Vec<u32> = to_delete
            .iter()
            .map(|&i| self.partitions[i].partition_number)
            .collect();
        let fs_arg = self.target_fs.diskpart_arg().to_string();
        let convert_cmd = active_scheme.diskpart_cmd().map(|s| s.to_string());
        let unit_val = self.cluster_size.diskpart_unit();

        self.is_formatting = true;
        self.progress_val = 0.10;
        self.progress_step = "Lancement de diskpart...".into();
        self.status = Status::Info("Formatage en cours, ne débranchez pas le périphérique...".into());

        let (tx, rx): (Sender<FormatMsg>, Receiver<FormatMsg>) = channel();
        self.rx_msg = Some(rx);

        std::thread::spawn(move || {
            let res = run_diskpart_pipeline(
                disk_number,
                &partition_numbers,
                all_selected,
                convert_cmd.as_deref(),
                &fs_arg,
                unit_val,
                &clean_label,
                &tx,
            );
            let _ = tx.send(FormatMsg::Done(res));
        });
    }

    fn check_formatting_status(&mut self) {
        let mut finished_result = None;
        if let Some(rx) = &self.rx_msg {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    FormatMsg::Progress(pct, step) => {
                        self.progress_val = pct;
                        self.progress_step = step;
                    }
                    FormatMsg::Done(res) => {
                        finished_result = Some(res);
                    }
                }
            }
        }

        if let Some(result) = finished_result {
            self.is_formatting = false;
            self.rx_msg = None;
            self.safety_confirmed = false;

            match result {
                Ok(_) => {
                    self.progress_val = 1.0;
                    self.refresh_drives();
                    self.status = Status::Success("✅ Disque formaté avec succès.".into());
                }
                Err(e) => {
                    self.status = Status::Error(format!("Erreur lors du formatage : {}", e));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UI egui
// ---------------------------------------------------------------------------
impl eframe::App for RazDiskApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.is_formatting {
            self.check_formatting_status();
            ctx.request_repaint();
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(243, 243, 243))
                    .inner_margin(20.0),
            )
            .show(ctx, |ui| {
                // --- SÉLECTION DU DISQUE & INFOS MATÉRIELLES ---
                ui.add_enabled_ui(!self.is_formatting, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Lecteur :").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("🔄 Actualiser").clicked() {
                                self.refresh_drives();
                            }
                        });
                    });

                    let current_drive = self
                        .selected_drive
                        .map(|i| self.drives[i].label())
                        .unwrap_or_else(|| "— Sélectionner un disque —".into());

                    let mut clicked_drive: Option<usize> = None;
                    egui::ComboBox::from_id_salt("drive_combo")
                        .selected_text(current_drive)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (i, d) in self.drives.iter().enumerate() {
                                if ui
                                    .selectable_label(self.selected_drive == Some(i), d.label())
                                    .clicked()
                                {
                                    clicked_drive = Some(i);
                                }
                            }
                        });
                    if let Some(i) = clicked_drive {
                        self.selected_drive = Some(i);
                        self.load_partitions();
                    }

                    // Encart d'informations détaillées du disque sélectionné
                    if let Some(idx) = self.selected_drive {
                        let d = &self.drives[idx];
                        let total_gb = d.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                        let space_info = if let Some(free) = d.free_bytes {
                            let free_gb = free as f64 / (1024.0 * 1024.0 * 1024.0);
                            format!("{:.2} Go libres / {:.2} Go totaux", free_gb, total_gb)
                        } else {
                            format!("{:.2} Go totaux (volume non monté)", total_gb)
                        };

                        ui.add_space(4.0);
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(232, 238, 245))
                            .rounding(4.0)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Modèle : {} (PhysicalDrive{})",
                                        d.model_name, d.physical_drive
                                    ))
                                    .small(),
                                );
                                ui.label(
                                    egui::RichText::new(format!("Capacité : {}", space_info)).small(),
                                );
                            });
                    }
                });

                ui.add_space(8.0);

                // --- PARTITIONS ---
                ui.add_enabled_ui(!self.is_formatting, |ui| {
                    ui.label(egui::RichText::new("Partitions à supprimer :").strong());
                    egui::Frame::none()
                        .fill(egui::Color32::WHITE)
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgb(200, 200, 200),
                        ))
                        .rounding(4.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if self.partitions.is_empty() {
                                ui.colored_label(
                                    egui::Color32::GRAY,
                                    "Aucune partition sélectionnée.",
                                );
                            } else {
                                egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                                    for (i, p) in self.partitions.iter().enumerate() {
                                        ui.checkbox(
                                            &mut self.selected_to_delete[i],
                                            format!(
                                                "Partition #{} [{}] — {:.1} Mo",
                                                p.partition_number,
                                                p.style,
                                                p.size_mb()
                                            ),
                                        );
                                    }
                                });
                                ui.separator();
                                ui.horizontal(|ui| {
                                    if ui.button("Tout cocher").clicked() {
                                        self.selected_to_delete.iter_mut().for_each(|s| *s = true);
                                    }
                                    if ui.button("Tout décocher").clicked() {
                                        self.selected_to_delete.iter_mut().for_each(|s| *s = false);
                                    }
                                });
                            }
                        });
                });

                ui.add_space(10.0);

                // --- OPTIONS DE FORMATAGE ---
                ui.add_enabled_ui(!self.is_formatting, |ui| {
                    let all_selected = !self.selected_to_delete.is_empty()
                        && self.selected_to_delete.iter().all(|&b| b);

                    ui.columns(2, |cols| {
                        cols[0].label(egui::RichText::new("Système de fichiers :").strong());
                        egui::ComboBox::from_id_salt("fs_combo")
                            .selected_text(self.target_fs.display())
                            .width(cols[0].available_width())
                            .show_ui(&mut cols[0], |ui| {
                                ui.selectable_value(
                                    &mut self.target_fs,
                                    FileSystemChoice::ExFat,
                                    "exFAT",
                                );
                                ui.selectable_value(
                                    &mut self.target_fs,
                                    FileSystemChoice::Ntfs,
                                    "NTFS",
                                );
                                ui.selectable_value(
                                    &mut self.target_fs,
                                    FileSystemChoice::Fat32,
                                    "FAT32",
                                );
                            });

                        cols[1].label(egui::RichText::new("Schéma de partition :").strong());
                        cols[1].add_enabled_ui(all_selected, |ui| {
                            let scheme_text = if all_selected {
                                self.partition_scheme.display()
                            } else {
                                "Conserver (Tout cocher requis)"
                            };
                            egui::ComboBox::from_id_salt("scheme_combo")
                                .selected_text(scheme_text)
                                .width(ui.available_width())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.partition_scheme,
                                        PartitionSchemeChoice::Keep,
                                        PartitionSchemeChoice::Keep.display(),
                                    );
                                    ui.selectable_value(
                                        &mut self.partition_scheme,
                                        PartitionSchemeChoice::Mbr,
                                        PartitionSchemeChoice::Mbr.display(),
                                    );
                                    ui.selectable_value(
                                        &mut self.partition_scheme,
                                        PartitionSchemeChoice::Gpt,
                                        PartitionSchemeChoice::Gpt.display(),
                                    );
                                });
                        });
                    });

                    ui.add_space(6.0);

                    ui.columns(2, |cols| {
                        cols[0].label(egui::RichText::new("Nom de volume :").strong());
                        cols[0].add(
                            egui::TextEdit::singleline(&mut self.partition_label)
                                .desired_width(cols[0].available_width())
                                .char_limit(self.target_fs.max_label_len())
                                .hint_text("RAZDISK"),
                        );

                        cols[1].label(egui::RichText::new("Unité d'allocation :").strong());
                        egui::ComboBox::from_id_salt("cluster_combo")
                            .selected_text(self.cluster_size.display())
                            .width(cols[1].available_width())
                            .show_ui(&mut cols[1], |ui| {
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Default,
                                    ClusterSizeChoice::Default.display(),
                                );
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Kb4,
                                    ClusterSizeChoice::Kb4.display(),
                                );
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Kb8,
                                    ClusterSizeChoice::Kb8.display(),
                                );
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Kb16,
                                    ClusterSizeChoice::Kb16.display(),
                                );
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Kb32,
                                    ClusterSizeChoice::Kb32.display(),
                                );
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Kb64,
                                    ClusterSizeChoice::Kb64.display(),
                                );
                                ui.selectable_value(
                                    &mut self.cluster_size,
                                    ClusterSizeChoice::Kb128,
                                    ClusterSizeChoice::Kb128.display(),
                                );
                            });
                    });

                    ui.add_space(8.0);

                    // Sécurité anti-erreur renforcée
                    let has_selection = self.selected_drive.is_some()
                        && self.selected_to_delete.iter().any(|&b| b);
                    ui.add_enabled_ui(has_selection, |ui| {
                        ui.checkbox(
                            &mut self.safety_confirmed,
                            egui::RichText::new(
                                "Je confirme vouloir effacer définitivement les partitions cochées.",
                            )
                            .color(egui::Color32::from_rgb(170, 40, 40)),
                        );
                    });
                });

                ui.add_space(10.0);

                // --- BARRE DE PROGRESSION & STATUT ---
                if self.is_formatting {
                    ui.add(
                        egui::ProgressBar::new(self.progress_val)
                            .show_percentage()
                            .animate(true)
                            .text(&self.progress_step),
                    );
                    ui.add_space(6.0);
                }

                match &self.status {
                    Status::None => {}
                    Status::Info(s) => {
                        ui.colored_label(egui::Color32::from_rgb(50, 100, 180), s);
                    }
                    Status::Success(s) => {
                        ui.colored_label(egui::Color32::from_rgb(40, 140, 70), s);
                    }
                    Status::Error(s) => {
                        egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                            ui.colored_label(egui::Color32::from_rgb(200, 60, 60), s);
                        });
                    }
                }

                // --- BOUTONS ---
                ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                    ui.horizontal(|ui| {
                        let close_btn = ui.add_enabled(
                            !self.is_formatting,
                            egui::Button::new("Fermer").min_size(egui::vec2(80.0, 30.0)),
                        );
                        if close_btn.clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }

                        let can_act = !self.is_formatting
                            && self.selected_drive.is_some()
                            && !self.partitions.is_empty()
                            && self.selected_to_delete.iter().any(|&b| b)
                            && self.safety_confirmed;

                        ui.add_enabled_ui(can_act, |ui| {
                            let btn = egui::Button::new(
                                egui::RichText::new("Démarrer")
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(100.0, 30.0))
                            .fill(egui::Color32::from_rgb(0, 120, 215));

                            if ui.add(btn).clicked() {
                                self.execute();
                            }
                        });
                    });
                });

                // --- DEBUG ---
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.collapsing("🔍 Debug", |ui| {
                        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                            for d in &self.all_drives {
                                ui.label(format!(
                                    "PhysicalDrive{} [{}] ({}) — {}",
                                    d.physical_number,
                                    d.bus_label,
                                    d.model_name,
                                    if d.accepted { "✅" } else { "⛔" }
                                ));
                            }
                        });
                    });
                });
            });
    }
}

// ---------------------------------------------------------------------------
// Fonctions utilitaires
// ---------------------------------------------------------------------------
fn sanitize_label(label: &str, max_len: usize) -> String {
    let s: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .take(max_len)
        .collect();
    let trimmed = s.trim();
    if trimmed.is_empty() {
        "RAZDISK".to_string()
    } else {
        trimmed.to_string()
    }
}

fn write_script(path: &std::path::Path, script: &str) -> std::io::Result<()> {
    std::fs::write(path, script)
}

fn get_partitions(physical_drive: u32) -> Result<Vec<PartitionInfo>, String> {
    let path = format!(r"\\.\PhysicalDrive{}", physical_drive);
    let file = File::open(&path).map_err(|e| format!("Ouverture: {}", e))?;
    let handle = HANDLE(file.as_raw_handle() as _);

    let mut buffer = vec![0u8; 65536];
    let mut bytes_returned = 0u32;

    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
            None,
            0,
            Some(buffer.as_mut_ptr() as *mut _),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if ok.is_err() {
        return Err(format!("IOCTL échoué: {:?}", ok));
    }
    if bytes_returned < std::mem::size_of::<DRIVE_LAYOUT_INFORMATION_EX>() as u32 {
        return Ok(Vec::new());
    }

    let layout = unsafe { &*(buffer.as_ptr() as *const DRIVE_LAYOUT_INFORMATION_EX) };
    let count = layout.PartitionCount;
    let mut parts = Vec::new();
    let base_ptr = layout.PartitionEntry.as_ptr();

    for i in 0..count as usize {
        let part_ptr = unsafe { base_ptr.add(i) };
        let part = unsafe { &*part_ptr };
        if part.PartitionLength <= 0 {
            continue;
        }

        let style_str = if part.PartitionStyle == PARTITION_STYLE_MBR {
            "MBR"
        } else if part.PartitionStyle == PARTITION_STYLE_GPT {
            "GPT"
        } else {
            "?"
        };

        parts.push(PartitionInfo {
            partition_number: part.PartitionNumber,
            start_lba: (part.StartingOffset / 512) as u64,
            sectors: (part.PartitionLength / 512) as u64,
            style: style_str.to_string(),
        });
    }
    Ok(parts)
}

fn run_diskpart_pipeline(
    disk_number: u32,
    partition_numbers: &[u32],
    all_selected: bool,
    convert_cmd: Option<&str>,
    fs: &str,
    unit: Option<u32>,
    label: &str,
    tx: &Sender<FormatMsg>,
) -> Result<(), String> {
    let _ = tx.send(FormatMsg::Progress(0.30, "Exécution des commandes diskpart...".into()));

    let mut dp_script = String::new();
    dp_script.push_str(&format!("select disk {}\n", disk_number));

    if all_selected {
        dp_script.push_str("clean\n");
        if let Some(cmd) = convert_cmd {
            dp_script.push_str(&format!("{}\n", cmd));
        }
    } else {
        for &p in partition_numbers {
            dp_script.push_str(&format!("select partition {}\n", p));
            dp_script.push_str("delete partition override\n");
        }
    }

    dp_script.push_str("create partition primary\n");
    dp_script.push_str(&format!("format fs={} label=\"{}\"", fs, label));
    if let Some(u) = unit {
        dp_script.push_str(&format!(" unit={}", u));
    }
    dp_script.push_str(" quick\n");
    dp_script.push_str("assign\n");
    dp_script.push_str("exit\n");

    let script_path = std::env::temp_dir().join("razdisk_dp.txt");
    write_script(&script_path, &dp_script).map_err(|e| format!("Écriture script: {}", e))?;

    let output = Command::new("diskpart")
        .arg("/s")
        .arg(&script_path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Exécution diskpart: {}", e))?;

    let _ = std::fs::remove_file(&script_path);

    if output.status.success() {
        let _ = tx.send(FormatMsg::Progress(1.0, "Terminé !".into()));
        Ok(())
    } else {
        let (out, _, _) = encoding_rs::WINDOWS_1252.decode(&output.stdout);
        Err(format!("Échec diskpart :\n{}", out))
    }
}

fn enumerate_all_drives() -> (Vec<DebugDriveInfo>, Vec<RemovableDrive>) {
    let mut all = Vec::new();
    let mut accepted = Vec::new();

    for n in 0..16u32 {
        let path = format!(r"\\.\PhysicalDrive{}", n);
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let handle = HANDLE(file.as_raw_handle() as _);

        let (bus_type, bus_label, model_name, bus_err) = match query_device_info_handle(handle) {
            Ok((b, model)) => (Some(b), bus_label_of(b).to_string(), model, String::new()),
            Err(e) => (None, "Inconnu".to_string(), "Disque générique".to_string(), e),
        };

        let size_bytes = query_length_handle(handle).unwrap_or(0);
        let letter = find_letter_for_physical_drive(n);
        let free_bytes = letter.and_then(query_free_space);

        let (is_accepted, reason) = if let Some(bus) = bus_type {
            if bus == 0x07 || bus == 0x0C || bus == 0x0D || bus == 0x03 || bus == 0x04 {
                (true, String::from("bus externe"))
            } else {
                (false, format!("bus interne ({})", bus_label))
            }
        } else {
            (false, format!("échec bus: {}", bus_err))
        };

        let info = DebugDriveInfo {
            physical_number: n,
            bus_type,
            bus_label,
            model_name: model_name.clone(),
            letter,
            size_bytes,
            accepted: is_accepted,
            reason,
        };

        if is_accepted {
            accepted.push(RemovableDrive {
                letter,
                physical_drive: n,
                model_name,
                size_bytes,
                free_bytes,
                bus_type: bus_type.unwrap(),
            });
        }
        all.push(info);
    }
    (all, accepted)
}

fn query_device_info_handle(handle: HANDLE) -> Result<(u32, String), String> {
    let query = StoragePropertyQuery {
        property_id: STORAGE_DEVICE_PROPERTY,
        query_type: 0,
        additional_parameters: [0; 4],
    };
    let mut buffer = [0u8; 512];
    let mut bytes_returned = 0u32;

    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const _),
            std::mem::size_of::<StoragePropertyQuery>() as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    match ok {
        Ok(_) => {
            if bytes_returned < 36 {
                return Err(format!("bytes_returned={} (trop petit)", bytes_returned));
            }
            let desc = unsafe { &*(buffer.as_ptr() as *const StorageDeviceDescriptor) };

            let vendor = read_cstr_at(&buffer, desc.vendor_id_offset as usize);
            let product = read_cstr_at(&buffer, desc.product_id_offset as usize);

            let full_name = format!("{} {}", vendor, product).trim().to_string();
            let model = if full_name.is_empty() {
                "Périphérique USB".to_string()
            } else {
                full_name
            };

            Ok((desc.bus_type, model))
        }
        Err(e) => Err(format!("OS error 0x{:08X}", e.code().0)),
    }
}

fn read_cstr_at(buf: &[u8], offset: usize) -> String {
    if offset == 0 || offset >= buf.len() {
        return String::new();
    }
    if let Ok(cstr) = CStr::from_bytes_until_nul(&buf[offset..]) {
        cstr.to_string_lossy().trim().to_string()
    } else {
        String::new()
    }
}

fn query_free_space(letter: char) -> Option<u64> {
    let root: Vec<u16> = format!(r"{}:\", letter)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut free_bytes_available = 0u64;
    let mut total_number_of_bytes = 0u64;
    let mut total_number_of_free_bytes = 0u64;

    let ok = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(root.as_ptr()),
            Some(&mut free_bytes_available),
            Some(&mut total_number_of_bytes),
            Some(&mut total_number_of_free_bytes),
        )
        .is_ok()
    };

    if ok {
        Some(free_bytes_available)
    } else {
        None
    }
}

fn query_length_handle(handle: HANDLE) -> Option<u64> {
    let mut info: GetLengthInformation = unsafe { std::mem::zeroed() };
    let mut bytes = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_LENGTH_INFO,
            None,
            0,
            Some(&mut info as *mut _ as *mut _),
            std::mem::size_of::<GetLengthInformation>() as u32,
            Some(&mut bytes),
            None,
        )
        .is_ok()
    };
    if ok && info.length > 0 {
        Some(info.length as u64)
    } else {
        None
    }
}

fn find_letter_for_physical_drive(physical_number: u32) -> Option<char> {
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        if get_physical_drive_number(letter) == Some(physical_number) {
            return Some(letter);
        }
    }
    None
}

fn get_physical_drive_number(letter: char) -> Option<u32> {
    let path = format!(r"\\.\{}:", letter);
    let file = File::open(&path).ok()?;
    let handle = HANDLE(file.as_raw_handle() as _);
    let mut info: StorageDeviceNumber = unsafe { std::mem::zeroed() };
    let mut bytes_returned = 0u32;

    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            0,
            Some(&mut info as *mut _ as *mut _),
            std::mem::size_of::<StorageDeviceNumber>() as u32,
            Some(&mut bytes_returned),
            None,
        )
        .is_ok()
    };
    if ok {
        Some(info.device_number)
    } else {
        None
    }
}