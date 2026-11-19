#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::ffi::CStr;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives};
use windows::Win32::System::Ioctl::{
    DRIVE_LAYOUT_INFORMATION_EX, PARTITION_STYLE_GPT, PARTITION_STYLE_MBR,
};
use windows::Win32::System::IO::DeviceIoControl;

// ---------------------------------------------------------------------------
// Langue de l'interface (FR / EN)
// ---------------------------------------------------------------------------
static LANG: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Fr,
    En,
}

fn lang() -> Lang {
    if LANG.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        Lang::Fr
    } else {
        Lang::En
    }
}

fn set_lang(l: Lang) {
    LANG.store(
        if l == Lang::Fr { 0 } else { 1 },
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Langue de l'interface Windows : français si l'identifiant de langue principal est 0x0C.
fn detect_lang() -> Lang {
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if id & 0x3FF == 0x0C {
        Lang::Fr
    } else {
        Lang::En
    }
}

/// `tr!("français", "english")` : renvoie le texte dans la langue courante.
macro_rules! tr {
    ($fr:expr, $en:expr) => {
        if lang() == Lang::Fr {
            $fr
        } else {
            $en
        }
    };
}

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
/// Icône de la fenêtre (même dessin que `icon.ico`) : carré sombre, disque clair percé, barre rouge « remise à zéro ».
fn window_icon_rgba(n: u32) -> Vec<u8> {
    // Couleur d'un point en coordonnées 0..1 (RGBA, transparent hors de l'icône).
    fn sample(x: f32, y: f32) -> [f32; 4] {
        // Carré arrondi (rayon 0,2)
        let r = 0.2_f32;
        let dx = (x - 0.5).abs() - (0.5 - r);
        let dy = (y - 0.5).abs() - (0.5 - r);
        let outside = dx.max(0.0).hypot(dy.max(0.0)) > r;
        if outside {
            return [0.0, 0.0, 0.0, 0.0];
        }
        // Barre rouge en diagonale
        let (ax, ay, bx, by) = (0.30_f32, 0.70_f32, 0.70_f32, 0.30_f32);
        let (abx, aby) = (bx - ax, by - ay);
        let t = (((x - ax) * abx + (y - ay) * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
        let d = ((x - (ax + t * abx)).powi(2) + (y - (ay + t * aby)).powi(2)).sqrt();
        if d <= 0.04 {
            return [230.0, 70.0, 70.0, 255.0];
        }
        // Disque clair avec trou central
        let dist = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
        if dist <= 0.13 {
            return [40.0, 44.0, 60.0, 255.0];
        }
        if dist <= 0.32 {
            return [235.0, 238.0, 245.0, 255.0];
        }
        [40.0, 44.0, 60.0, 255.0]
    }

    // Suréchantillonnage 3x3 pour lisser les bords.
    const SS: u32 = 3;
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for j in 0..n {
        for i in 0..n {
            let mut acc = [0.0_f32; 4];
            for sj in 0..SS {
                for si in 0..SS {
                    let x = (i as f32 + (si as f32 + 0.5) / SS as f32) / n as f32;
                    let y = (j as f32 + (sj as f32 + 0.5) / SS as f32) / n as f32;
                    let c = sample(x, y);
                    // moyenne pondérée par l'opacité
                    acc[0] += c[0] * c[3];
                    acc[1] += c[1] * c[3];
                    acc[2] += c[2] * c[3];
                    acc[3] += c[3];
                }
            }
            let a = acc[3];
            if a > 0.0 {
                out.extend_from_slice(&[
                    (acc[0] / a) as u8,
                    (acc[1] / a) as u8,
                    (acc[2] / a) as u8,
                    (a / (SS * SS) as f32) as u8,
                ]);
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    out
}

fn main() -> Result<(), eframe::Error> {
    set_lang(detect_lang());
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::IsUserAnAdmin;
        if !unsafe { IsUserAnAdmin().as_bool() } {
            rfd::MessageDialog::new()
                .set_title(tr!(
                    "RazDisk — Privilèges requis",
                    "RazDisk — Privileges required"
                ))
                .set_description(tr!(
                    "RazDisk nécessite des droits administrateur.",
                    "RazDisk requires administrator rights."
                ))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Ok(());
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([490.0, 720.0])
            .with_min_inner_size([440.0, 650.0])
            .with_title("RazDisk")
            .with_icon(egui::IconData {
                rgba: window_icon_rgba(64),
                width: 64,
                height: 64,
            }),
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
            Self::Fat32 => tr!("FAT32 (Sans limite de taille)", "FAT32 (no size limit)"),
            Self::ExFat => "exFAT",
            Self::Ntfs => "NTFS",
        }
    }
    fn format_arg(&self) -> &'static str {
        match self {
            Self::Fat32 => "FAT32",
            Self::ExFat => "exFAT",
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
            Self::Keep => tr!("Conserver (Actuel)", "Keep (current)"),
            Self::Mbr => tr!("MBR (BIOS / Compatibilité)", "MBR (BIOS / compatibility)"),
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
            Self::Default => tr!("Par défaut", "Default"),
            Self::Kb4 => tr!("4 Ko (4096 octets)", "4 KB (4096 bytes)"),
            Self::Kb8 => tr!("8 Ko", "8 KB"),
            Self::Kb16 => tr!("16 Ko", "16 KB"),
            Self::Kb32 => tr!("32 Ko", "32 KB"),
            Self::Kb64 => tr!("64 Ko", "64 KB"),
            Self::Kb128 => tr!("128 Ko", "128 KB"),
        }
    }
    fn format_unit(&self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Kb4 => Some("4K"),
            Self::Kb8 => Some("8K"),
            Self::Kb16 => Some("16K"),
            Self::Kb32 => Some("32K"),
            Self::Kb64 => Some("64K"),
            Self::Kb128 => Some("128K"),
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
            .unwrap_or_else(|| tr!("Sans lettre", "No letter").into());
        let size_gb = self.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        format!(
            "{} — {} ({:.1} {}) [{}]",
            letter_str,
            self.model_name,
            size_gb,
            tr!("Go", "GB"),
            bus_label_of(self.bus_type)
        )
    }
}

fn bus_label_of(bus: u32) -> &'static str {
    match bus {
        0x00 => "SCSI",
        0x01 => "ATAPI",
        0x02 => "ATA",
        0x03 => "1394",
        0x04 => "FireWire",
        0x05 => "SSA",
        0x06 => "Fibre",
        0x07 => "USB",
        0x08 => "RAID",
        0x09 => "iSCSI",
        0x0A => "SAS",
        0x0B => "SATA",
        0x0C => "SD",
        0x0D => "MMC",
        0x0E => "Virtual",
        0x0F => "VHD",
        0x10 => "Spaces",
        0x11 => "NVMe",
        0x12 => "SCM",
        0x13 => "UFS",
        _ => tr!("Inconnu", "Unknown"),
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
            "{}{}",
            self.drives.len(),
            tr!(
                " disque(s) externe(s) détecté(s).",
                " external drive(s) detected."
            )
        ));
    }

    fn load_partitions(&mut self) {
        let Some(idx) = self.selected_drive else {
            return;
        };
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
                    "{}{}",
                    self.partitions.len(),
                    tr!(" partition(s) trouvée(s).", " partition(s) found.")
                ));
            }
            Err(e) => {
                self.status =
                    Status::Error(format!("{} : {}", tr!("Erreur lecture", "Read error"), e));
            }
        }
    }

    fn execute(&mut self) {
        let Some(idx) = self.selected_drive else {
            return;
        };
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
            self.status = Status::Error(
                tr!(
                    "Cochez au moins une partition à supprimer.",
                    "Tick at least one partition to delete."
                )
                .into(),
            );
            return;
        }

        let all_selected = self.selected_to_delete.iter().all(|&b| b);
        let active_scheme = if all_selected {
            self.partition_scheme
        } else {
            PartitionSchemeChoice::Keep
        };

        let clean_label = sanitize_label(&self.partition_label, self.target_fs.max_label_len());

        let msg_tpl = tr!(
            "⚠️  DERNIÈRE CONFIRMATION\n\n\
             • Cible : {}\n\
             • Action : Supprimer {} partition(s)\n\
             • Schéma : {}\n\
             • Format : {} ({})\n\
             • Nom : '{}'\n\n\
             TOUTES LES DONNÉES seront DÉFINITIVEMENT PERDUES.\n\n\
             Lancer le formatage ?",
            "⚠️  FINAL CONFIRMATION\n\n\
             • Target: {}\n\
             • Action: delete {} partition(s)\n\
             • Scheme: {}\n\
             • Format: {} ({})\n\
             • Label: '{}'\n\n\
             ALL DATA will be PERMANENTLY LOST.\n\n\
             Start formatting?"
        );
        let msg = fill(
            msg_tpl,
            &[
                drive_label.to_string(),
                to_delete.len().to_string(),
                active_scheme.display().to_string(),
                self.target_fs.display().to_string(),
                self.cluster_size.display().to_string(),
                clean_label.to_string(),
            ],
        );

        let confirmed = rfd::MessageDialog::new()
            .set_title(tr!("Confirmer le formatage", "Confirm formatting"))
            .set_description(&msg)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_level(rfd::MessageLevel::Warning)
            .show();

        if confirmed != rfd::MessageDialogResult::Yes {
            self.status = Status::Info(tr!("Opération annulée.", "Operation cancelled.").into());
            return;
        }

        let partition_numbers: Vec<u32> = to_delete
            .iter()
            .map(|&i| self.partitions[i].partition_number)
            .collect();
        let fs_arg = self.target_fs.format_arg().to_string();
        let convert_cmd = active_scheme.diskpart_cmd().map(|s| s.to_string());
        let unit_arg = self.cluster_size.format_unit().map(|s| s.to_string());
        let drive_letter = drive.letter;

        self.is_formatting = true;
        self.progress_val = 0.05;
        self.progress_step = tr!("Initialisation du nettoyage...", "Starting cleanup...").into();
        self.status = Status::Info(
            tr!(
                "Formatage en cours, ne débranchez pas le périphérique...",
                "Formatting in progress, do not unplug the device..."
            )
            .into(),
        );

        let (tx, rx): (Sender<FormatMsg>, Receiver<FormatMsg>) = channel();
        self.rx_msg = Some(rx);

        std::thread::spawn(move || {
            let res = run_format_pipeline(
                disk_number,
                drive_letter,
                &partition_numbers,
                all_selected,
                convert_cmd.as_deref(),
                &fs_arg,
                unit_arg.as_deref(),
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
                    self.status = Status::Success(
                        tr!(
                            "✅ Disque formaté avec succès.",
                            "✅ Disk formatted successfully."
                        )
                        .into(),
                    );
                }
                Err(e) => {
                    self.status = Status::Error(format!(
                        "{} : {}",
                        tr!("Erreur lors du formatage", "Formatting error"),
                        e
                    ));
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

        let mut new_lang: Option<Lang> = None;
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
                        ui.label(egui::RichText::new(tr!("Lecteur :", "Drive:")).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tr!("🔄 Actualiser", "🔄 Refresh")).clicked() {
                                self.refresh_drives();
                            }
                            let mut l = lang();
                            ui.selectable_value(&mut l, Lang::En, "EN");
                            ui.selectable_value(&mut l, Lang::Fr, "FR");
                            if l != lang() {
                                new_lang = Some(l);
                            }
                        });
                    });

                    let current_drive = self
                        .selected_drive
                        .map(|i| self.drives[i].label())
                        .unwrap_or_else(|| tr!("— Sélectionner un disque —", "— Select a drive —").into());

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
                            format!(
                                "{:.2} {} / {:.2} {}",
                                free_gb,
                                tr!("Go libres", "GB free"),
                                total_gb,
                                tr!("Go totaux", "GB total")
                            )
                        } else {
                            format!(
                                "{:.2} {}",
                                total_gb,
                                tr!("Go totaux (volume non monté)", "GB total (volume not mounted)")
                            )
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
                                        "{} : {} (PhysicalDrive{})",
                                        tr!("Modèle", "Model"),
                                        d.model_name,
                                        d.physical_drive
                                    ))
                                    .small(),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{} : {}", tr!("Capacité", "Capacity"), space_info))
                                        .small(),
                                );
                            });
                    }
                });

                ui.add_space(8.0);

                // --- PARTITIONS ---
                ui.add_enabled_ui(!self.is_formatting, |ui| {
                    ui.label(egui::RichText::new(tr!("Partitions à supprimer :", "Partitions to delete:")).strong());
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
                                    tr!("Aucune partition sélectionnée.", "No partition selected."),
                                );
                            } else {
                                egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                                    for (i, p) in self.partitions.iter().enumerate() {
                                        ui.checkbox(
                                            &mut self.selected_to_delete[i],
                                            format!(
                                                "Partition #{} [{}] — {:.1} {}",
                                                p.partition_number,
                                                p.style,
                                                p.size_mb(),
                                                tr!("Mo", "MB")
                                            ),
                                        );
                                    }
                                });
                                ui.separator();
                                ui.horizontal(|ui| {
                                    if ui.button(tr!("Tout cocher", "Select all")).clicked() {
                                        self.selected_to_delete.iter_mut().for_each(|s| *s = true);
                                    }
                                    if ui.button(tr!("Tout décocher", "Select none")).clicked() {
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
                        cols[0].label(egui::RichText::new(tr!("Système de fichiers :", "File system:")).strong());
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
                                    FileSystemChoice::Fat32.display(),
                                );
                            });

                        cols[1].label(egui::RichText::new(tr!("Schéma de partition :", "Partition scheme:")).strong());
                        cols[1].add_enabled_ui(all_selected, |ui| {
                            let scheme_text = if all_selected {
                                self.partition_scheme.display()
                            } else {
                                tr!("Conserver (Tout cocher requis)", "Keep (select all required)")
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
                        cols[0].label(egui::RichText::new(tr!("Nom de volume :", "Volume label:")).strong());
                        cols[0].add(
                            egui::TextEdit::singleline(&mut self.partition_label)
                                .desired_width(cols[0].available_width())
                                .char_limit(self.target_fs.max_label_len())
                                .hint_text("RAZDISK"),
                        );

                        cols[1].label(egui::RichText::new(tr!("Unité d'allocation :", "Allocation unit size:")).strong());
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
                            egui::RichText::new(tr!(
                                "Je confirme vouloir effacer définitivement les partitions cochées.",
                                "I confirm I want to permanently erase the ticked partitions."
                            ))
                            .color(egui::Color32::from_rgb(170, 40, 40)),
                        );
                    });
                });

                ui.add_space(10.0);

                // --- BARRE DE PROGRESSION RÉELLE & STATUT ---
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

                // --- BOUTONS (Alignés en bas à droite) ---
                ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                    ui.horizontal(|ui| {
                        let close_btn = ui.add_enabled(
                            !self.is_formatting,
                            egui::Button::new(tr!("Fermer", "Close")).min_size(egui::vec2(80.0, 30.0)),
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
                                egui::RichText::new(tr!("Démarrer", "Start"))
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
                    ui.collapsing(tr!("🔍 Debug", "🔍 Debug"), |ui| {
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

        if let Some(l) = new_lang {
            set_lang(l);
            // Les messages déjà affichés étaient dans l'ancienne langue : on les efface.
            if !self.is_formatting {
                self.status = Status::None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fonctions utilitaires
// ---------------------------------------------------------------------------

/// Remplace successivement chaque `{}` du modèle par les valeurs données.
fn fill(template: &str, values: &[String]) -> String {
    let mut out = String::new();
    let mut it = values.iter();
    let mut rest = template;
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if let Some(v) = it.next() {
            out.push_str(v);
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

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

/// Décode la sortie console Windows (page de code OEM 850 en France) ; `encoding_rs` ne la propose pas.
fn decode_oem850(bytes: &[u8]) -> String {
    const HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜø£Ø×ƒáíóúñÑªº¿®¬½¼¡«»░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐└┴┬├─┼ãÃ╚╔╩╦╠═╬¤ðÐÊËÈıÍÎÏ┘┌█▄¦Ì▀ÓßÔÒõÕµþÞÚÛÙýÝ¯´­±‗¾¶§÷¸°¨·¹³²■ ";
    let high: Vec<char> = HIGH.chars().collect();
    bytes
        .iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                high[(b - 0x80) as usize]
            }
        })
        .collect()
}

/// Sortie de diskpart sans l'en-tête (version, copyright, nom de l'ordinateur) : il ne reste que les messages utiles.
fn diskpart_summary(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = decode_oem850(stdout);
    if !stderr.is_empty() {
        text.push('\n');
        text.push_str(&decode_oem850(stderr));
    }
    text.lines()
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with("Microsoft DiskPart")
                && !l.starts_with("Copyright")
                && !l.starts_with("Sur l'ordinateur")
                && !l.starts_with("On computer")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exécute un script diskpart ; renvoie une erreur lisible (messages utiles + commandes exécutées).
fn run_diskpart(script: &str) -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let script_path =
        std::env::temp_dir().join(format!("razdisk_dp_{}_{}.txt", std::process::id(), nanos));
    write_script(&script_path, script)
        .map_err(|e| format!("{}: {}", tr!("Écriture du script", "Script write error"), e))?;

    let output = Command::new("diskpart")
        .arg("/s")
        .arg(&script_path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| {
            format!(
                "{}: {}",
                tr!("Exécution de diskpart", "Running diskpart"),
                e
            )
        })?;

    let _ = std::fs::remove_file(&script_path);

    let summary = diskpart_summary(&output.stdout, &output.stderr);
    // diskpart peut renvoyer un code de réussite malgré une erreur : on cherche aussi le message d'erreur dans le texte.
    let reported_error =
        summary.contains("rencontré une erreur") || summary.contains("encountered an error");
    if !output.status.success() || reported_error {
        return Err(format!(
            "{} {:?}) :\n{}\n\n{} :\n{}",
            tr!("Échec diskpart (code", "diskpart failed (code"),
            output.status.code(),
            summary,
            tr!("Commandes exécutées", "Commands executed"),
            script.trim_end().replace('\n', " → ")
        ));
    }
    Ok(())
}

/// Attend (au plus `tries` × 0,5 s) que la partition `partition` du disque `disk` ait une lettre de lecteur.
fn wait_for_letter(disk: u32, partition: u32, tries: u32) -> Option<char> {
    for _ in 0..tries {
        if let Some(l) = letter_for_partition(disk, partition) {
            return Some(l);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    None
}

fn letter_for_partition(disk: u32, partition: u32) -> Option<char> {
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        if get_device_numbers(letter) == Some((disk, partition)) {
            return Some(letter);
        }
    }
    None
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
        return Err(format!("{}: {:?}", tr!("IOCTL échoué", "IOCTL failed"), ok));
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

#[allow(clippy::too_many_arguments)]
fn run_format_pipeline(
    disk_number: u32,
    _old_letter: Option<char>,
    partition_numbers: &[u32],
    all_selected: bool,
    convert_cmd: Option<&str>,
    fs: &str,
    unit: Option<&str>,
    label: &str,
    tx: &Sender<FormatMsg>,
) -> Result<(), String> {
    let _ = tx.send(FormatMsg::Progress(
        0.10,
        tr!(
            "Préparation du disque (diskpart)...",
            "Preparing the disk (diskpart)..."
        )
        .into(),
    ));

    // Partitions présentes avant l'opération. Celles qui ne sont pas supprimées sont conservées :
    // on les reconnaît ensuite par leur position de départ pour ne jamais formater l'une d'elles par erreur.
    let before = get_partitions(disk_number).unwrap_or_default();
    let kept_starts: Vec<u64> = before
        .iter()
        .filter(|p| !partition_numbers.contains(&p.partition_number))
        .map(|p| p.start_lba)
        .collect();

    let mut dp_script = String::new();
    dp_script.push_str(&format!("select disk {}\n", disk_number));

    if all_selected {
        dp_script.push_str("clean\n");
        if let Some(cmd) = convert_cmd {
            dp_script.push_str(&format!("{}\n", cmd));
        }
    } else {
        // Suppression par numéros décroissants : après chaque suppression, diskpart peut renuméroter les suivantes.
        let mut numbers = partition_numbers.to_vec();
        numbers.sort_unstable_by(|a, b| b.cmp(a));
        for p in numbers {
            dp_script.push_str(&format!("select partition {}\n", p));
            dp_script.push_str("delete partition override\n");
        }
    }

    // Pas de `assign` ici : sur un support amovible, Windows attribue la lettre tout seul, et après
    // `create partition` aucun volume n'est sélectionné, donc `assign` échouait (« Aucun volume n'est spécifié »).
    dp_script.push_str("create partition primary\n");
    dp_script.push_str("exit\n");

    run_diskpart(&dp_script)?;

    let _ = tx.send(FormatMsg::Progress(
        0.40,
        tr!(
            "Recherche de la nouvelle partition...",
            "Looking for the new partition..."
        )
        .into(),
    ));
    std::thread::sleep(std::time::Duration::from_millis(800));

    let after = get_partitions(disk_number).map_err(|e| {
        format!(
            "{}: {}",
            tr!("Lecture des partitions", "Reading partitions"),
            e
        )
    })?;
    let new_partition = after
        .iter()
        .find(|p| !kept_starts.contains(&p.start_lba))
        .map(|p| p.partition_number)
        .ok_or_else(|| {
            tr!(
                "La nouvelle partition est introuvable après sa création.",
                "The new partition could not be found after its creation."
            )
            .to_string()
        })?;

    let mut found = wait_for_letter(disk_number, new_partition, 12);
    if found.is_none() {
        // Windows n'a pas attribué de lettre : on l'attribue explicitement à cette partition (et à elle seule).
        let assign_script = format!(
            "select disk {}\nselect partition {}\nassign\nexit\n",
            disk_number, new_partition
        );
        let _ = run_diskpart(&assign_script);
        found = wait_for_letter(disk_number, new_partition, 10);
    }
    let Some(letter) = found else {
        return Err(tr!(
            "Impossible de déterminer la lettre de la nouvelle partition. Formatez-la depuis l'Explorateur Windows.",
            "Could not determine the drive letter of the new partition. Format it from Windows Explorer."
        )
        .into());
    };

    let _ = tx.send(FormatMsg::Progress(
        0.50,
        format!(
            "{} {}: — {}...",
            tr!("Formatage du lecteur", "Formatting drive"),
            letter,
            fs
        ),
    ));

    // `format` est un programme « .com » : Rust ne cherche que les « .exe » (d'où « program not found »).
    // On l'appelle par son chemin complet dans System32.
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let format_path = std::path::Path::new(&system_root)
        .join("System32")
        .join("format.com");
    let mut cmd = Command::new(format_path);
    cmd.arg(format!("{}:", letter))
        .arg(format!("/FS:{}", fs))
        .arg("/Q")
        .arg(format!("/V:{}", label))
        .arg("/Y");

    if fs == "FAT32" && unit.is_none() {
        cmd.arg("/A:32K");
    } else if let Some(u) = unit {
        cmd.arg(format!("/A:{}", u));
    }

    let mut fmt_child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("{}: {}", tr!("Lancement de format", "Starting format"), e))?;

    let mut fmt_output = String::new();
    if let Some(stdout) = fmt_child.stdout.take() {
        let reader = BufReader::new(stdout);
        // La sortie de `format` est en Windows-1252 (pas en UTF-8) et sa progression utilise des retours chariot (\r) :
        // on lit donc des octets bruts, découpés sur \n puis \r, et on décode chaque morceau.
        for chunk in reader.split(b'\n') {
            let Ok(bytes) = chunk else { continue };
            for part in bytes.split(|b| *b == b'\r') {
                let decoded = decode_oem850(part);
                let trimmed = decoded.trim();
                if trimmed.is_empty() {
                    continue;
                }
                fmt_output.push_str(trimmed);
                fmt_output.push('\n');

                if trimmed.contains('%') {
                    if let Some(pct) = extract_percentage(trimmed) {
                        let mapped = 0.50 + (pct / 100.0) * 0.45;
                        let _ = tx.send(FormatMsg::Progress(
                            mapped,
                            format!("{} {} : {:.0} %", tr!("Formatage", "Formatting"), fs, pct),
                        ));
                    }
                }
            }
        }
    }

    let status = fmt_child
        .wait()
        .map_err(|e| format!("{}: {}", tr!("Attente de format", "Waiting for format"), e))?;

    if status.success() {
        let _ = tx.send(FormatMsg::Progress(1.0, tr!("Terminé !", "Done!").into()));
        Ok(())
    } else {
        Err(format!(
            "{} {:?}) :\n{}",
            tr!("Échec du formatage (code", "Formatting failed (code"),
            status.code(),
            fmt_output
        ))
    }
}

fn extract_percentage(line: &str) -> Option<f32> {
    for word in line.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_ascii_digit());
        if let Ok(val) = clean.parse::<f32>() {
            if (0.0..=100.0).contains(&val) {
                return Some(val);
            }
        }
    }
    None
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
            Err(e) => (
                None,
                tr!("Inconnu", "Unknown").to_string(),
                tr!("Disque générique", "Generic disk").to_string(),
                e,
            ),
        };

        let size_bytes = query_length_handle(handle).unwrap_or(0);
        let letter = find_letter_for_physical_drive(n);
        let free_bytes = letter.and_then(query_free_space);

        let (is_accepted, reason) = if let Some(bus) = bus_type {
            if bus == 0x07 || bus == 0x0C || bus == 0x0D || bus == 0x03 || bus == 0x04 {
                (true, String::from(tr!("bus externe", "external bus")))
            } else {
                (
                    false,
                    format!("{} ({})", tr!("bus interne", "internal bus"), bus_label),
                )
            }
        } else {
            (
                false,
                format!("{}: {}", tr!("échec bus", "bus error"), bus_err),
            )
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
                tr!("Périphérique USB", "USB device").to_string()
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
    let root: Vec<u16> = format!("{}:\\", letter)
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
    get_device_numbers(letter).map(|(disk, _)| disk)
}

/// (numéro de disque physique, numéro de partition) d'une lettre de lecteur.
fn get_device_numbers(letter: char) -> Option<(u32, u32)> {
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
        Some((info.device_number, info.partition_number))
    } else {
        None
    }
}
