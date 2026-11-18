# RazDisk — High-Performance Windows Storage Utility

> **Méthodologie :** Pilotage itératif et prototypage accéléré assisté par IA  
> **Cible :** Administrateurs système, techniciens et utilisateurs avancés sous Windows  

---

## 🎯 Vue d'ensemble & Objectif stratégique

**RazDisk** répond à une problématique opérationnelle récurrente sous Windows : la rigidité des outils natifs pour la gestion, le nettoyage et le formatage des supports amovibles (clés USB, cartes SD, disques externes). 

L'objectif de ce projet est de livrer un utilitaire de bureau robuste, portable et sécurisé, combinant la performance bas niveau de **Rust** à une interface graphique moderne et réactive (`egui`).

---

## 📐 Périmètre Fonctionnel & Spécifications

* **Sécurité & Filtrage Matériel :** 
  * Isolation stricte des bus externes (USB, SD, MMC) pour interdire toute interaction accidentelle avec les disques internes du système.
  * Double validation obligatoire avant exécution (check-box de consentement + boîte de dialogue modale récapitulative).
* **Gestion Avancée des Partitions :** 
  * Suppression de partitions récalcitrantes (idéal pour le nettoyage de supports amorçables multi-boot ou Linux/macOS).
  * Conversion dynamique des schémas de partition (`MBR` pour le mode BIOS/rétrocompatibilité, `GPT` pour l'UEFI et les volumes > 2 To).
* **Formats & Paramétrage Fin :** 
  * Support multi-format : **exFAT**, **NTFS** et **FAT32** standard.
  * Personnalisation du nom de volume (gestion des contraintes de caractères selon le système de fichiers) et choix de la taille des unités d'allocation (clusters).
* **Monitoring en Temps Réel :** 
  * Architecture multithreadée (`mpsc` channels) garantissant une interface fluide pendant les traitements lourds.
  * Suivi transparent des étapes de configuration et de formatage.

---

## 🛠️ Stack Technique & Architecture

* **Langage principal :** Rust (Edition 2021) pour sa sûreté mémoire et ses performances d'exécution.
* **Interface Graphique :** `eframe` / `egui` (IHM épurée, ergonomique et sans dépendance lourde).
* **Interactions Bas Niveau (OS) :** Crate `windows` (manipulation directe des IOCTL de stockage, requêtes de propriétés de périphériques et structures de disques).
* **Automatisation Système :** Pilotage sécurisé et en arrière-plan des scripts `diskpart` (sans affichage de console parasite, subsystem Windows configuré).

---

## 🔄 Cycle de Vie & Méthodologie de Livraison

Ce projet s'inscrit dans un processus de développement basé sur un cycle court :

1. **Cadrage du besoin :** Identification des points de friction des utilitaires natifs Windows.
2. **Spécification technique :** Conception modulaire du pipeline d'exécution et des barrières de sécurité.
3. **Prototypage & Implémentation :** Génération, tests unitaires et intégration continue des composants Rust.
4. **Recette & Validation :** Tests en conditions réelles sous environnement Windows (validation des cas limites et gestion des codes de retour).

---

## 📦 Installation & Utilisation

### Prérequis
* Système d'exploitation : Windows 10 / 11 (x64)
* Environnement Rust (Cargo) installé.
* Privilèges **Administrateur** requis (l'exécutable intègre un manifeste de sécurité pour l'élévation automatique des droits).

### Compilation

# Cloner le dépôt
```bash
git clone [https://github.com/ChatCarbo/razdisk.git](https://github.com/ChatCarbo/razdisk.git)
cd razdisk
```

# Compiler l'exécutable optimisé en mode release
```bash
cargo build --release
```

# Avertissement de Sécurité
Cet outil effectue des modifications directes sur les tables de partitions des supports physiques connectés. Une mauvaise manipulation sur un disque cible peut entraîner une perte définitive de données. L'utilisateur est seul responsable de la vérification de la sélection du lecteur avant toute opération de formatage.
