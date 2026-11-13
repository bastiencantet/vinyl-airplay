# Vinyl AirPlay — communauté

**Tes vinyles, tes enceintes. Ton téléphone reste libre.**

Un émetteur AirPlay en Rust pour envoyer une entrée audio Linux vers des HomePod,
avec contrôle depuis un navigateur et connexion automatique à la lecture.

C'est une **version alpha communautaire** : l'audio fonctionne sur du matériel
réel, mais l'installation doit encore être validée chez d'autres personnes.
Le Raspberry Pi est une cible à tester, pas encore une compatibilité acquise.

## Essayer

Prérequis : Linux, Rust/Cargo, `alsa-utils`, Python 3.10+ pour l'interface web,
une entrée audio stéréo de niveau ligne, et les enceintes sur le même réseau.

```sh
git clone https://github.com/bastiencantet/vinyl-airplay.git
cd vinyl-airplay
cargo build --release --locked
arecord -L
./target/release/vinyl-airplay discover --seconds 5
python3 scripts/serve-web.py --alsa-device 'plughw:CARD=USB,DEV=0'
```

Remplace le périphérique ALSA par le tien, puis ouvre http://127.0.0.1:8787/.
Sélectionne une enceinte, applique et démarre. Sur une installation neuve, la
diffusion et la reconnaissance des morceaux sont désactivées.

Pour ton téléphone, ajoute `--listen` avec l'adresse IPv4 privée de la machine
Linux. Le contrôle est alors accessible aux appareils du réseau de confiance :
ne publie pas ce port sur Internet.

## Ce qu'il faut savoir

- Une entrée micro USB n'est pas forcément une entrée ligne stéréo adaptée.
- La platine doit sortir en LINE ou passer par un préampli phono.
- Le transport actuel est en PCM stéréo 16 bits / 44,1 kHz.
- Le délai demandé n'est pas une mesure de latence acoustique.
- La connexion automatique peut manquer le début d'un morceau pendant la négociation.
- La reconnaissance des titres est facultative et contacte un service externe.
- La partie TV et le pilote matériel expérimental ne font pas partie de cette alpha.

[Installation détaillée](SETUP.md) · [Compatibilité](COMPATIBILITY.md) ·
[Contribuer](../CONTRIBUTING.md) · [Feuille de route](../ROADMAP.md)

Licence GPL-3.0-or-later. Projet indépendant, sans affiliation avec Apple.
