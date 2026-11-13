# Steam Press — Guide d'utilisation

## 1. Installation des outils

Steam Press n'embarque ni SteamCMD ni 7-Zip.

```bash
# SteamCMD (Linux)
mkdir -p ~/steamcmd && cd ~/steamcmd
curl -sqL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz | tar zxvf -
./steamcmd.sh +quit
# Puis ajoutez ~/steamcmd au PATH, ou renseignez steamcmdpath dans les réglages.

# 7-Zip
sudo apt install p7zip-full      # fournit 7z
```

Pour RAR, fournissez vous-même un binaire `rar` légalement obtenu (RAR est propriétaire).
Pour DepotDownloader, placez le binaire `DepotDownloader` dans le `PATH` ou indiquez
`depotdownloaderpath`.

## 2. Première configuration

```bash
cargo build --release -p steam-press-cli
./target/release/steam-press login          # ou : login --username anonymous
./target/release/steam-press doctor
./target/release/steam-press depots sync
```

`doctor` doit afficher les chemins de SteamCMD, 7-Zip, le nombre de dépôts connus et un
dossier de sortie inscriptible.

## 3. File de jeux

```bash
steam-press queue add 2379780 --os lin64 --branch public
steam-press queue add 123456 --os win64 --branch beta --branch-password secret
steam-press queue list
steam-press queue move 2 1
steam-press queue export ./ma-file.sspq
steam-press queue import ./ma-file.sspq
```

Format `.SSPQ`, compatible SuperSteamPacker :

```
lin64|2379780|public|
win64|123456|beta|secret
```

Les lignes vides, commentaires (`;`, `#`) et lignes incomplètes sont tolérés à l'import.

## 4. Téléchargement et compression

```bash
# Préflight sans téléchargement
steam-press pack --appid 2379780 --os lin64 --anonymous --dry-run

# Téléchargement + 7z par défaut
steam-press pack --appid 2379780 --os lin64

# RAR, sortie personnalisée, moteur DepotDownloader
steam-press pack --appid 2379780 --os win64 --compressor rar \
  --downloader depotdownloader --output /mnt/games/Packs

# Sans compression (dossier conservé)
steam-press pack --queue ~/.config/steam-press/queue.sspq --no-compress
```

Nommage de sortie, identique à SuperSteamPacker :

```
<Jeu>.Build.<buildid>.<OS>.<branche>.7z
<Jeu>.Build.<buildid>.<OS>.<branche>.rar
<Jeu>.Build.<buildid>.<OS>.<branche>/   (sans compression)
```

Les options 7-Zip par défaut sont `-mx9 -sdel -p<mot de passe> -mhe=on -v5g`.
Si un seul volume `.7z.001` est produit, il est renommé en `.7z`.

Un job dont l'archive existe déjà est ignoré. `--compressor none` équivaut à
`--no-compress`.

## 5. Atelier

```bash
steam-press workshop add 730 123456
steam-press workshop list
steam-press workshop run --queue ~/.config/steam-press/queue.sswq
```

Format `.SSWQ` : `appid|workshopid`. L'import accepte aussi les lignes
`workshop_download_item <appid> <id> validate`. La sortie s'appelle
`Workshop_<appid>_<id>.7z`.

## 6. Manifestes et vie privée

Après téléchargement, Steam Press :

1. déplace `steamapps`/`depotcache` dans un dossier temporaire ;
2. supprime `libraryfolders.vdf`, `downloading/`, `temp/` et les fichiers Workshop
   temporaires ;
3. remet `LastOwner` et `LauncherPath` à `"0"` dans l'`.acf` existant ;
4. génère un `.acf` complet si le moteur n'en a pas fourni ;
5. calcule les lignes `depot - nom [Manifest id]`.

Un `.acf` généré contient `StateFlags 4`, `LastUpdated "0"`, `LastPlayed "0"`,
`LastOwner "0"`, `InstalledDepots`, `UserConfig` et `MountedConfig`.

## 7. Templates

Créez `mon-template.txt`, puis renseignez son chemin dans les réglages (ou `template` dans
`settings.ini`).

```
[url=][color=white][b]{{game_name}} [{{os}}] [Branch: {{branch}}] (Clean Steam Files)[/b][/color][/url]
[size=85][color=white][b]Version:[/b] [i]{{build_datetime_utc}} [Build {{build_id}}][/i][/color][/size]

[spoiler="Depots & Manifests"][code=text]{{#depots}}{{depot_id}} - {{depot_name}} [Manifest {{manifest_id}}]
{{/depots}}[/code][/spoiler]

Uploaded by {{username}} on {{upload_date}}.
```

Test rapide sans Steam :

```bash
steam-press template render ./job.json ./mon-template.txt
```

`job.json` accepte les champs `game_name`, `os`, `branch`, `build_id`, `build_datetime_utc`,
`app_id`, `description`, `website`, `username`, `filehost`, `upload_date` et `depots`
(tableau de `depot_id`, `depot_name`, `manifest_id`).

## 8. Réglages

Les clés de `settings.ini` restent compatibles avec SuperSteamPacker :

```ini
[SSP]
compressor=7z
customcompressoption=
skipcompression=0
darkmode=1
uploadcrewmode=0
rinruusername=
filehost=
outputdir=
steamcmdpath=
depotdownloaderpath=
sevenzippath=
rarpath=
compresspassword=cs.rin.ru
template=
key=<clé locale>
```

## 9. Erreurs fréquentes

| Message | Cause / solution |
|---|---|
| `SteamCMD introuvable` | installez SteamCMD ou renseignez `steamcmdpath` |
| `ce compte ne possède pas ce jeu` | le compte doit posséder l'app (`No subscription`) |
| `Steam limite temporairement les téléchargements` | attendez avant de relancer (`Rate Limit Exceeded`) |
| `identifiants Steam invalides ou Steam Guard requis` | relancez `steam-press login`, validez Steam Guard dans SteamCMD |
| `7-Zip introuvable` | installez `p7zip-full` ou renseignez `sevenzippath` |
| `aucun identifiant enregistré` | `steam-press login` ou `--anonymous` |

## 10. Sécurité

- Les identifiants sont chiffrés en AES-256-GCM avec une clé générée localement.
- Le fichier `credentials.bin` est créé en mode `0600`.
- Les mots de passe ne sont jamais journalisés et ne sont pas passés en argument de
  processus à DepotDownloader.
- Le script SteamCMD temporaire est lui aussi créé en `0600` puis supprimé.
