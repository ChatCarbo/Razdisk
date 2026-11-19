# RazDisk — High-Performance Windows Storage Utility

> **Méthodologie :** Pilotage itératif et prototypage accéléré assisté par IA  
> **Cible :** Administrateurs système, techniciens et utilisateurs avancés sous Windows  
> **Langues de l'interface :** Français (FR) / Anglais (EN)  

---

## 🎯 Vue d'ensemble & Objectif stratégique

**RazDisk** répond à une problématique opérationnelle récurrente sous Windows : la rigidité et les limitations des outils natifs pour la gestion, le nettoyage complet et le formatage des supports amovibles (clés USB, cartes SD, disques externes).

L'objectif de ce projet est de livrer un utilitaire de bureau robuste, portable et sécurisé, combinant la performance bas niveau de **Rust** à une interface graphique moderne et réactive (`egui`), tout en offrant une alternative moderne et maintenue aux anciens utilitaires du marché.

---

## 📐 Périmètre Fonctionnel & Spécifications

- **Interface Bilingue (EN / FR) :**
  - Bascule instantanée entre le français et l'anglais pour l'ensemble de l'interface et des messages d'exécution.
- **Sécurité & Filtrage Matériel :**
  - Isolation stricte des bus externes (USB, SD, MMC) via requêtes IOCTL afin d'interdire toute interaction accidentelle avec les disques internes du système.
  - Double validation obligatoire avant exécution (case de consentement explicite + boîte de dialogue modale récapitulative).
- **Gestion Avancée des Partitions :**
  - Suppression de partitions récalcitrantes ou cachées (idéal pour remettre à neuf des supports amorçables multi-boot, Ventoy, Linux ou macOS).
  - Conversion dynamique des schémas de partition : **`MBR`** (mode BIOS / rétrocompatibilité) et **`GPT`** (mode UEFI / volumes > 2 To).
- **Formats & Paramétrage Fin (FAT32 sans limite) :**
  - Support multi-format : **exFAT**, **NTFS** et **FAT32 sans limite de taille** (contournement de la barrière native des 32 Go de Windows pour les supports de grande capacité).
  - Pilotage natif de l'utilitaire système `format` pour l'initialisation fiable des systèmes de fichiers.
  - Personnalisation du nom de volume (validation automatique des contraintes de caractères selon le système de fichiers) et sélection fine de la taille des unités d'allocation (clusters).
- **Monitoring en Temps Réel :**
  - Architecture multithreadée (`mpsc` channels) garantissant une interface fluide et réactive pendant les opérations disque.
  - Suivi détaillé et transparent des étapes de nettoyage, de partitionnement et de formatage.

---

## ⚖️ Positionnement & Comparaison : RazDisk vs HP USB Disk Storage Format Tool

L'utilitaire historique de HP (*HP USB Disk Storage Format Tool*, v2.2.3) a longtemps servi de référence rapide sous Windows pour formater des clés USB en FAT32 au-delà de 32 Go. **RazDisk** reprend cette simplicité d'accès tout en modernisant l'architecture, la sécurité et les capacités de remise à zéro :

| Critère | RazDisk | Outil HP (*HP USB Disk Storage Format Tool*) |
| :--- | :--- | :--- |
| **Statut & Maintenance** | Projet actif, moderne, compatible Windows 10 / 11 | Utilitaire *legacy* abandonné (comportements erratiques sur les versions récentes de Windows) |
| **Langues** | **Bilingue (EN / FR)** | Anglais uniquement |
| **FAT32 Grande Capacité** | **FAT32 sans limite de taille** | Supporte le FAT32 > 32 Go, mais échoue souvent sur des disques déjà partitionnés |
| **Nettoyage & Partitions** | Suppression complète des tables/partitions récalcitrantes + conversion **MBR / GPT** | Formatage simple d'un volume existant uniquement (ne nettoie pas les partitions multiples) |
| **Sécurité Anti-Erreur** | Filtrage strict des bus amovibles (USB/SD/MMC) + **double validation** | Sélection directe sans garde-fous structurels avancés |
| **Interface & Fluidité** | IHM moderne `egui`, exécution multithreadée sans gel d'écran, élévation Admin intégrée | Fenêtre classique Windows XP/Vista sujette aux blocages pendant le formatage |
| **Transparence** | Code source Rust ouvert, auditable et évolutif | Binaire propriétaire fermé |

---

## 🛠️ Stack Technique & Architecture

- **Langage principal :** Rust (Edition 2021) pour sa sûreté mémoire, sa gestion stricte des erreurs et ses performances natives.
- **Interface Graphique :** `eframe` / `egui` (IHM légère, ergonomique et sans dépendance externe lourde).
- **Interactions Bas Niveau (OS) :** Crate `windows` (manipulation directe des IOCTL de stockage, interrogation des descripteurs de périphériques et des propriétés de bus).
- **Automatisation Système :** Orchestration sécurisée et silencieuse de `diskpart` et `format` en arrière-plan (sans fenêtre de console parasite, subsystem Windows configuré).

---

## 🔄 Cycle de Vie & Méthodologie de Livraison

Ce projet s'inscrit dans un processus de développement basé sur un cycle court :

1. **Cadrage du besoin :** Identification des limites des utilitaires natifs Windows et des anciens outils tiers.
2. **Spécification technique :** Conception modulaire du pipeline d'exécution (`diskpart` + `format`) et des barrières de sécurité matérielle.
3. **Prototypage & Implémentation :** Développement, tests et intégration continue des composants Rust et de l'internationalisation (EN/FR).
4. **Recette & Validation :** Tests sur supports physiques variés sous Windows (clés USB haute capacité, cartes SD, supports multi-partitions).

---

## 📦 Installation & Compilation

### Prérequis
- **Système d'exploitation :** Windows 10 / 11 (x64)
- **Toolchain :** Environnement Rust (`cargo` / `rustc`) installé.
- **Droits :** Privilèges **Administrateur** requis (l'exécutable embarque un manifeste Windows demandant automatiquement l'élévation UAC).

### Étapes de compilation

```bash
# Cloner le dépôt
git clone https://github.com/ChatCarbo/razdisk.git
cd razdisk

# Compiler l'exécutable optimisé en mode release
cargo build --release
```

L'exécutable généré se trouve dans `target\release\razdisk.exe`.

---

## ⚠️ Avertissement de Sécurité

Cet outil effectue des modifications directes sur les tables de partitions et les systèmes de fichiers des supports physiques connectés. Une mauvaise sélection de lecteur entraînera une **perte définitive des données** présentes sur le support cible. L'utilisateur demeure seul responsable de la vérification du périphérique sélectionné avant de confirmer l'opération.