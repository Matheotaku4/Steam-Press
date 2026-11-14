# Validation réelle — 1.0.1

Date : 14 novembre 2026  
Plateforme : Ubuntu 24.04.5, x86_64, sans compte Steam.

## Jeu et moteur

| Élément | Valeur |
|---|---|
| Application | Spacewar (`480`) |
| Dépôt | `481` |
| Manifeste | `3183503801510301321` |
| Build | `3538192` |
| Taille API | 1 906 055 octets |
| Moteur | DepotDownloader 3.4.0, connexion anonyme |
| Compresseur | 7-Zip 25.01 |

## Résultats

- Téléchargement anonyme réussi : 8 fichiers, 797 632 octets transférés.
- `pack --downloader depotdownloader --anonymous --no-compress` terminé avec le code 0.
- Contenu placé dans `steamapps/common/Spacewar/`, taille réelle 1 906 055 octets.
- ACF généré : `installdir=Spacewar`, `buildid=3538192`, `SizeOnDisk=1906055`,
  `LastOwner=0`.
- `pack --compressor 7z` terminé avec le code 0, archive de 605 520 octets.
- `7zz t -pcs.rin.ru` : « Everything is Ok ».
- Le dossier `.DepotDownloader` n'apparaît pas dans la sortie.

## Bug trouvé et corrigé en 1.0.1

Avant le correctif, DepotDownloader écrivait les fichiers à la racine de
`steamapps/`, produisait `SizeOnDisk=0` et laissait fuiter `.DepotDownloader`
dans l'archive. Le moteur recrée maintenant la disposition Steam
(`steamapps/common/<installdir>`) et supprime les métadonnées du sidecar.

## Comparaison avec les fixtures

Les fixtures ACF de Balatro contiennent `SizeOnDisk=1024` et
`BytesToDownload=1024`; l'ACF généré utilise `SizeOnDisk` réel et laisse
volontairement `BytesToDownload`/`BytesDownloaded` à `0` (ces compteurs sont
recalculés par Steam). Les champs `LastOwner` et `LauncherPath` sont nettoyés
ou absents, conformément au comportement de génération.

## Limites

SteamCMD n'a pas pu être exécuté réellement : son binaire Linux est en 32 bits
et l'environnement n'a pas de chargeur `ld-linux.so.2`. Les scénarios Steam
Guard, mauvais mot de passe, branche protégée et coupure réseau restent à
tester avec un compte réel.
