# RazDisk — High-Performance Windows Storage Utility

> **Methodology:** Iterative steering and AI-assisted rapid prototyping  
> **Target Audience:** System administrators, IT technicians, and power users on Windows  
> **Interface Languages:** English (EN) / French (FR)  
> 🇫🇷 *[Version française disponible dans README.md](README.md)*

---

## 🎯 Overview & Strategic Objective

**RazDisk** addresses a recurring operational pain point on Windows: the rigidity and built-in limitations of native utilities when managing, deep-cleaning, and formatting removable storage media (USB flash drives, SD cards, external drives).

The goal of this project is to deliver a robust, portable, and safe desktop utility combining the low-level performance of **Rust** with a modern, responsive GUI (`egui`), providing a maintained modern alternative to legacy formatting tools.

---

## 📐 Functional Scope & Specifications

- **Bilingual Interface (EN / FR):**
  - Instant switching between English and French across the entire UI and execution logs.
- **Hardware Filtering & Safety Guardrails:**
  - Strict isolation of external buses (`USB`, `SD`, `MMC`) via low-level IOCTL queries to prevent any accidental interaction with internal system drives.
  - Mandatory two-step confirmation before execution (explicit consent checkbox + summary modal dialog).
- **Advanced Partition Management:**
  - Deep removal of stubborn or hidden partitions (ideal for wiping multi-boot, Ventoy, Linux, or macOS bootable drives).
  - Dynamic partition scheme conversion: **`MBR`** (BIOS / legacy compatibility) and **`GPT`** (UEFI / volumes > 2 TB).
- **File Systems & Fine-Grained Formatting (Unlimited FAT32):**
  - Multi-filesystem support: **exFAT**, **NTFS**, and **FAT32 with no size limit** (bypassing Windows' native 32 GB restriction for high-capacity drives).
  - Native orchestration of the Windows `format` utility for reliable filesystem initialization.
  - Custom volume label input (with automatic character validation per filesystem) and fine-grained allocation unit (cluster) size selection.
- **Real-Time Monitoring:**
  - Multithreaded architecture (`mpsc` channels) ensuring a smooth, non-blocking UI during heavy disk operations.
  - Transparent step-by-step tracking of cleaning, partitioning, and formatting stages.

---

## ⚖️ Positioning & Comparison: RazDisk vs. HP USB Disk Storage Format Tool

The classic HP utility (*HP USB Disk Storage Format Tool*, v2.2.3) long served as a quick go-to tool on Windows for formatting USB drives in FAT32 beyond 32 GB. **RazDisk** preserves that straightforward workflow while modernizing the architecture, safety checks, and drive-wiping capabilities:

| Feature | RazDisk | HP USB Disk Storage Format Tool |
| :--- | :--- | :--- |
| **Status & Maintenance** | Actively maintained, built for Windows 10 / 11 | Abandoned *legacy* utility (prone to erratic behavior on modern Windows versions) |
| **Languages** | **Bilingual (EN / FR)** | English only |
| **Large-Capacity FAT32** | **FAT32 with no size limit** | Supports FAT32 > 32 GB, but often fails on pre-partitioned or multi-partition drives |
| **Wiping & Partitioning** | Full removal of stubborn partition tables + **MBR / GPT** conversion | Basic single-volume formatting only (cannot clean complex/hidden partition layouts) |
| **Anti-Error Safety** | Strict removable-bus filtering (`USB`/`SD`/`MMC`) + **two-step confirmation** | Direct drive selection with minimal structural guardrails |
| **UI & Responsiveness** | Modern `egui` interface, multithreaded execution (no UI freeze), built-in UAC elevation | Legacy Windows XP/Vista dialog prone to freezing during formatting |
| **Transparency** | Open, auditable, and extensible Rust source code | Closed-source proprietary binary |

---

## 🛠️ Tech Stack & Architecture

- **Core Language:** Rust (2021 Edition) for memory safety, strict error handling, and native performance.
- **Graphical User Interface:** `eframe` / `egui` (lightweight, ergonomic GUI with zero heavy runtime dependencies).
- **Low-Level OS Interactions:** `windows` crate (direct storage IOCTL manipulation, device descriptors, and bus property queries).
- **System Automation:** Secure, silent background orchestration of `diskpart` and `format` (no popup console windows, configured Windows GUI subsystem).

---

## 🔄 Lifecycle & Delivery Methodology

This project follows a short-cycle iterative development process:

1. **Needs Assessment:** Identifying friction points in native Windows utilities and outdated third-party tools.
2. **Technical Specification:** Modular design of the execution pipeline (`diskpart` + `format`) and hardware safety barriers.
3. **Prototyping & Implementation:** Rapid development, testing, and continuous integration of Rust components and EN/FR localization.
4. **QA & Validation:** Real-world hardware testing on Windows across high-capacity USB drives, SD cards, and multi-partition media.

---

## 📦 Installation & Build

### Prerequisites
- **Operating System:** Windows 10 / 11 (x64)
- **Toolchain:** Rust toolchain (`cargo` / `rustc`) installed.
- **Privileges:** **Administrator** rights required (the binary embeds a Windows application manifest requesting automatic UAC elevation).

### Build Instructions

```bash
# Clone the repository
git clone https://github.com/ChatCarbo/razdisk.git
cd razdisk

# Build the optimized release executable
cargo build --release
```

The compiled executable will be located at `target\release\razdisk.exe`.

---

## ⚠️ Safety Warning

This utility performs direct modifications to partition tables and file systems on connected physical storage devices. Selecting the wrong drive will result in **permanent data loss** on the target medium. The user is solely responsible for verifying the selected drive before confirming any formatting operation.