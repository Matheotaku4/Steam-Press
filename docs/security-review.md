# Revue de sécurité — coffre à identifiants

## Modèle de menace

Steam Press protège les identifiants Steam contre une lecture accidentelle ou
un vol de fichier isolé. Il ne protège pas contre un attaquant qui contrôle le
compte utilisateur, car la clé AES est stockée à côté du coffre dans
`settings.ini`.

## Implémentation

- AES-256-GCM avec une clé aléatoire de 32 octets générée par `OsRng`.
- Nonce aléatoire de 12 octets généré pour chaque chiffrement ; le nonce est
  préfixé au texte chiffré. Aucun nonce n'est réutilisé par le code.
- La clé est encodée en base64 dans `settings.ini` (`key=`). Le fichier de
  réglages n'est pas chiffré.
- `credentials.bin` est écrit en mode `0600` sur Unix. Sur Windows, les ACL
  du profil utilisateur s'appliquent.
- Le préfixe de format `SPV1:` permet de distinguer les anciens coffres 0.x.
- Les mots de passe ne sont jamais inclus dans les erreurs, les logs, l'historique
  de la GUI ou la sortie `--json`.
- DepotDownloader ne reçoit jamais le mot de passe en argument de processus ;
  SteamCMD le reçoit dans un script temporaire `0600`, supprimé après usage.

## Tests et cas d'échec

- Mauvais mot de passe, fichier tronqué et base64 invalide produisent une erreur
  explicite sans paniquer.
- Un coffre 0.x sans préfixe reste lisible.
- La perte de la clé rend le coffre illisible ; il faut se reconnecter.
- La suppression de `credentials.bin` est traitée comme l'absence de compte.

## Limites connues

- La clé et le coffre sont dans le même profil : un accès en lecture au profil
  permet le déchiffrement.
- Le coffre ne protège pas contre un keylogger ni contre un processus malveillant
  qui s'exécute sous le même compte.
- Les mises à jour automatiques vérifient un SHA-256 mais ne sont pas encore
  signées cryptographiquement ; la signature et la provenance SLSA sont suivies
  dans un ticket séparé.
- Les identifiants transitent inévitablement vers SteamCMD ou DepotDownloader
  pour l'authentification ; ces outils ont leur propre modèle de sécurité.
