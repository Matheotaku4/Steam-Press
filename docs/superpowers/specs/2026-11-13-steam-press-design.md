# Steam Press — Design

Date : 2026-11-13
Statut : approuvé pour implémentation

## Objectif

Réécrire en Rust [SuperSteamPacker](https://github.com/Masquerade64/SuperSteamPacker)
en reprenant les mécanismes éprouvés d'[OmniPacker](https://github.com/elgreams/OmniPacker).
Les deux dépôts sont des références **de logique**, pas d'interface : Steam Press a sa
propre interface native egui.

Steam Press prépare des archives de jeux Steam propres (« Clean Steam Files ») :
téléchargement des dépôts, assainissement des manifestes, compression et génération des
informations de publication cs.rin.ru.

Plateforme cible de la v1 : **Linux**. Le code reste portable (Windows/macOS plus tard),
aucun chemin codé en dur pour un seul OS.

## Décisions de cadrage

| Sujet | Décision |
|---|---|
| Interface | egui/eframe natif, pas de Tauri, pas de WebKit |
| Moteur de téléchargement | SteamCMD (défaut) **et** DepotDownloader derrière une abstraction |
| Périmètre | Parité SuperSteamPacker + extras OmniPacker (templates éditables, ACF généré, annulation propre) |
| Identifiants | Fichier local chiffré AES-256-GCM, clé locale ; jamais en clair |
| Vérification | Cœur testé unitairement + CLI exécutable ; GUI compilée mais non lancée ici (pas d'affichage) |
| Distribution | Binaires CLI et GUI séparés ; pas de packaging v1 |

## Architecture

Espace de travail Cargo à trois crates :

```
Cargo.toml                     workspace
crates/steam-press-core/       bibliothèque métier, aucune dépendance GUI
crates/steam-press-cli/        binaire `steam-press`, harnais réel et mode headless
crates/steam-press-gui/        binaire `steam-press-gui`, eframe/egui
docs/superpowers/specs/        cette spec
```

Règle de dépendance : `cli` et `gui` dépendent de `core` ; `core` ne connaît ni CLI ni GUI.
La GUI ne contient aucune logique métier : elle émet des commandes et consomme des
événements.

### Modules du cœur

| Module | Responsabilité |
|---|---|
| `config` | `settings.ini` compatible `[SSP]`, répertoires XDG, chemins d'outils, valeurs par défaut |
| `credentials` | coffre AES-256-GCM, génération/rotation de clé, effacement |
| `ini` | lecteur/écrivain INI tolérant (BOM, CRLF, commentaires, clés dupliquées) |
| `depots` | `depots.ini` (SteamDepotNames), synchronisation GitHub, fusion sans doublons |
| `steam` | appinfo `api.steamcmd.net`, store `appdetails`, normalisation des noms |
| `vdf` | parseur/sérialiseur KeyValues (ACF) |
| `acf` | lecture d'`appmanifest`, neutralisation, génération complète, redistribuables 228980 |
| `platform` | `Win64/Win32/Linux64/Linux32/Mac`, libellés, directives SteamCMD |
| `queue` | `.SSPQ` et `.SSWQ`, sérialisation compatible SuperSteamPacker, import tolérant |
| `downloader` | trait commun, types de requête/rapport, événements, annulation |
| `downloader::steamcmd` | scripts, processus, analyse de logs, installation de SteamCMD |
| `downloader::depotdownloader` | arguments par dépôt, sidecar, parsing de sortie |
| `archive` | 7z/RAR, options par défaut et personnalisées, volumes, détection d'existants |
| `template` | rendu intégré RIN/Upload Crew, moteur à jetons, boucle de dépôts |
| `naming` | nommage `<Jeu>.Build.<build>.<OS>.<branche>`, assainissement portable |
| `pack` | pipeline complet et reprise des étapes |
| `events` | `Progress`, `Stage`, `Log`, `Finished`, `Cancelled`, `Failed` |
| `tooling` | détection d'exécutables, ouverture de dossier, extraction zip |

### Dépendances prévues

`serde`, `serde_json`, `reqwest` (blocking, rustls), `chrono`, `aes-gcm`, `rand`,
`zip`, `eframe`/`egui`, `serde` pour les échanges. Pas de crate GTK/WebKit.
Les dépendances sont ajoutées au fil des besoins, pas par anticipation.

## Configuration et données

### Répertoires

- Config : `$XDG_CONFIG_HOME/steam-press/` (défaut `~/.config/steam-press/`)
- Données : `$XDG_DATA_HOME/steam-press/` (défaut `~/.local/share/steam-press/`)
- Surcharge : `STEAM_PRESS_HOME`
- Sortie par défaut : `~/SteamPress/Completed` ; `Temp/`, `SteamCMD/`, `DepotDownloader/`
  et `templates/` vivent sous `~/SteamPress/`. Tous ces chemins sont modifiables.

### `settings.ini`

Section `[SSP]`, clés compatibles avec SuperSteamPacker :

```
[SSP]
compressor=7z            ; 7z | RAR
customcompressoption=    ; vide = options SSP par défaut
skipcompression=0
darkmode=1
uploadcrewmode=0
rinruusername=
filehost=
outputdir=/home/user/SteamPress/Completed
steamcmdpath=
depotdownloaderpath=
sevenzippath=
rarpath=
compresspassword=cs.rin.ru
template=
key=<clé locale encodée base64>
```

### `credentials.bin`

AES-256-GCM, nonce aléatoire de 12 octets préfixé, base64 dans un fichier à permissions
`0600`. La clé est générée localement et conservée dans `settings.ini`. Aucun identifiant
n'est journalisé ni transmis ailleurs qu'au moteur de téléchargement.

### Files

- `.SSPQ` : une ligne `os|appid|branche|motdepasse`, formats `win64`, `win32`, `lin64`,
  `lin32`, `macos`. Lecture tolérante aux lignes vides, commentaires et lignes à 2–3 champs.
- `.SSWQ` : `appid|workshopid`, ou lignes `workshop_download_item <appid> <id> validate`.
- L'écriture reste strictement compatible avec SuperSteamPacker.

### Templates

Fichiers texte dans `templates/`, sélectionnés dans les réglages. Jetons :
`{{game_name}}`, `{{os}}`, `{{branch}}`, `{{build_id}}`, `{{build_datetime_utc}}`,
`{{upload_datetime_utc}}`, `{{app_id}}`, `{{description}}`, `{{website}}`,
`{{username}}`, `{{upload_date}}`, `{{filehost}}`, `{{depot_list}}`,
`{{primary_depot_id}}`, `{{primary_manifest_id}}`, et le bloc
`{{#depots}}...{{depot_id}}...{{depot_name}}...{{manifest_id}}...{{/depots}}`.
Sans template personnalisé, les rendus intégrés reproduisent le format
`[CS.RIN.RU Info]` et `[Upload Crew Info]` de SuperSteamPacker.

## Téléchargement

```rust
pub trait Downloader: Send {
    fn kind(&self) -> DownloaderKind;
    fn check(&self) -> Result<ToolStatus, DownloadError>;
    fn download(&mut self, req: &DownloadRequest, sink: &EventSink)
        -> Result<DownloadReport, DownloadError>;
    fn download_workshop(&mut self, req: &WorkshopRequest, sink: &EventSink)
        -> Result<WorkshopReport, DownloadError>;
    fn cancel(&mut self);
}
```

`DownloadRequest` porte : appid, plateforme, branche, mot de passe de branche, identifiants,
répertoire de travail, dépôts/manifests épinglés optionnels.

- **SteamCmd** : script temporaire à permissions restreintes, supprimé après exécution.
  `login`, directives de plateforme, `app_update <appid> [-beta <b>] [-betapassword <p>] validate`,
  `quit`. Analyse des logs `content_log.txt`/`connection_log.txt` :
  « No subscription », « Rate Limit Exceeded », « Invalid Password », absence de
  `sitelicense_steamcmd.txt`/`compat_log.txt` → code d'échec typé.
- **DepotDownloader** : un passage par appid avec `-app`, `-depot`, `-manifest`,
  `-branch`, `-branchpassword`, `-os`, `-dir`, `-validate`, `-username`/`-password`
  (ou session), sidecar détecté dans les réglages ou `PATH`.

Les deux implémentations renvoient un `DownloadReport` commun : `content_dir` (racine du
contenu téléchargé), `app_manifest` optionnel (ACF fourni par le moteur), `depots`
(id, nom, manifest utilisé) et `build_id`. SteamCMD renvoie `steamapps/` + `depotcache/` ;
DepotDownloader renvoie son répertoire de staging et ses manifestes. Le cœur normalise
ensuite les deux sorties (ACF existant assaini ou généré, contenu déplacé vers `Temp/`).

Annulation : drapeau atomique + terminaison du processus enfant ; l'archive partielle et
la sortie non compressée sont conservées, comme OmniPacker.

## Pipeline de pack

1. **Préflight** : appinfo (nom, `installdir`, buildid/timeupdated de la branche, dépôts) ;
   si `<Jeu>.Build.<build>.<OS>.<branche>.7z` (ou `.rar`, `.7z.001`, `.part1.rar`) existe,
   statut `Skipped`.
2. **Téléchargement** via le moteur choisi, avec événements de progression et logs.
3. **Nettoyage** : déplacement du contenu du `content_dir` et des manifestes vers `Temp/`,
   suppression de `libraryfolders.vdf`, `downloading/`, `temp/`, `workshop/downloads/`,
   `workshop/temp/` lorsqu'ils existent.
4. **Manifestes** : neutralisation `LastOwner`/`LauncherPath` si ACF existant sinon
   génération complète (appid, name, buildid, installdir, InstalledDepots, SharedDepots,
   UserConfig, MountedConfig) + `appmanifest_228980.acf` pour les redistribuables.
5. **Dépôts** : lignes `id - nom [Manifest id]` via `depots.ini`, fallback `DepotName`.
6. **Compression** : 7z (`a -mx9 -sdel -p<mdp> -mhe=on -v5g`) ou RAR
   (`a -df -hp<mdp> -htc -v5000000k -r`), ou déplacement du dossier si compression
   désactivée.
7. **Finalisation** : si un seul volume `.7z.001`, renommage en `.7z` ; suppression des
   fichiers de la sortie en cas d'échec de compression.
8. **Informations** : `[CS.RIN.RU Info] <stem>.txt`, `[Upload Crew Info] <stem>.txt` si
   activé, ou template personnalisé rendu.
9. **Événement final** : chemin de sortie, build, date, dépôts, durée.

Une étape en échec produit `Failed` avec un message typé, nettoie uniquement ses artefacts
temporaires et laisse les jobs suivants continuer.

## CLI

```
steam-press doctor
steam-press login [--username U] [--save]
steam-press info <appid> [--branch B]
steam-press queue list|add|remove|move|clear|import|export
steam-press pack [--queue FILE]... [--appid ID --os linux64 ...]
                  [--downloader steamcmd|depotdownloader]
                  [--compressor 7z|rar|none] [--output DIR] [--dry-run]
steam-press workshop add|import|export|run
steam-press depots sync|lookup <id>|list
steam-press template render --job FILE --template FILE
```

Sortie lisible sur stdout, événements de progression sur stderr pour ne pas polluer les
données redirigées. Codes de sortie : `0` succès, `1` échec métier, `2` usage invalide,
`130` annulation.

## GUI egui

Fenêtre unique, thème sombre par défaut, quatre onglets :

- **Jeu** : AppID, OS, branche + mot de passe, bouton Ajouter, file réordonnable, statut
  par ligne (Prêt / Téléchargement / Compression / Terminé / Ignoré / Échec), Démarrer,
  Annuler, progression et log repliable.
- **Atelier** : AppID + Workshop ID, file, mêmes contrôles.
- **Dépôts** : synchronisation `depots.ini`, recherche par ID ou nom, nombre d'entrées.
- **Réglages** : compresseur et options, mot de passe d'archive, compression désactivée,
  sortie, chemins SteamCMD/DepotDownloader/7z/RAR, identifiants et effacement, uploader,
  filehost, mode Upload Crew, template, thème.

Le worker tourne dans un thread dédié et communique par canal ; l'interface ne bloque
jamais sur un téléchargement. Fermer la fenêtre pendant un job demande confirmation et
propose l'annulation.

## Gestion des erreurs

Erreurs typées (`ConfigError`, `DownloadError`, `ArchiveError`, `TemplateError`,
`PipelineError`) avec contexte actionnable : outil manquant et chemin attendu, identifiants
invalides, branche protégée, quota Steam, espace disque insuffisant, archive existante.
Aucun `unwrap` sur une entrée utilisateur ou un fichier externe.

## Tests et vérification

Tests unitaires sans réseau ni Steam :

- INI : BOM, CRLF, doublons, sections, round-trip.
- Files : encodage/décodage SSP, tolérance, atelier.
- Plateformes : clés, libellés, directives.
- SteamCMD : scripts exacts (public, bêta, bêta protégée, atelier), classification des logs.
- VDF/ACF : parsing réel, neutralisation, génération, échappement des guillemets.
- Nommage : espaces, caractères interdits, accents, points finaux, cas réels.
- Archives : arguments 7z/RAR, options personnalisées, détection, finalisation de volume.
- Templates : rendu intégré, jetons, boucle de dépôts, échappement.
- Chiffrement : aller-retour, mauvaise clé, fichier corrompu.
- API : fixtures JSON appinfo/store, branches manquantes, réponses en erreur.
- Fusion de dépôts : doublons, « Unknown », tri.

Vérifications exécutées dans cet environnement : `cargo test --workspace`,
`cargo build -p steam-press-cli`, `cargo check -p steam-press-gui`, `cargo clippy`
si disponible. La GUI ne sera pas lancée (aucun affichage) ; les téléchargements réels ne
seront pas exécutés (identifiants Steam absents). Ces limites seront rapportées.

## Hors périmètre v1

- Téléchargements réels validés contre Steam (aucun compte disponible ici).
- Packaging AppImage/deb, mises à jour automatiques, signature.
- Connexion QR, Steam Guard interactif dans la GUI.
- Support Windows/macOS complet (code portable, validation plus tard).
- Onglets Achievements et Post Template « Coming soon » de SuperSteamPacker : non repris
  tels quels ; le moteur de templates couvre le besoin de publication.

## Jalons

1. Fondations du cœur : workspace, config, INI, plateformes, files, nommage, chiffrement.
2. Steam et manifestes : API, VDF/ACF, dépôts, génération ACF.
3. Moteur SteamCMD complet et classification des logs.
4. Pipeline de pack, archives, templates.
5. Moteur DepotDownloader et sélection.
6. CLI complète et tests d'intégration locaux.
7. GUI egui branchée sur le cœur.
8. Documentation d'utilisation et passe de vérification finale.
