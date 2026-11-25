# Mission : construire « Kadr », un éditeur vidéo open source en Rust, piloté par Claude via MCP

> Nom provisoire : **Kadr**. Remplace-le partout si je t'en donne un autre.

Tu es l'ingénieur principal de ce projet. Tu vas construire, phase par phase, un éditeur vidéo
**gratuit, open source et sans fonctionnalité premium**, écrit en **Rust**. Ses deux différences
avec les éditeurs du marché :

1. **Montage piloté depuis une conversation Claude** grâce à un serveur **MCP local** : dans
   Claude Desktop, je dis « prends la vidéo que je viens d'importer, coupe les silences, ajoute un
   titre au début et exporte en vertical » et Claude le fait dans le logiciel. **Aucune clé API,
   aucun coût en plus : un simple abonnement Claude suffit.**
2. **Floutage d'un visage choisi par un algorithme classique de suivi (pas d'IA, pas de réseau de
   neurones)** : je sélectionne le visage sur une image, le logiciel le suit sur tout le clip et
   le floute.

Le projet doit fonctionner d'abord sur **PC (Windows en priorité, puis macOS et Linux)**, puis être
porté en **APK Android** (et iOS plus tard) **sans réécrire le moteur**.

Je développe sous **Windows 11**. Mon shell principal est PowerShell.

---

## 0. Règles de travail (à respecter tout au long du projet)

- **Travaille phase par phase** (section 6). À la fin de chaque phase :
  1. tout compile (`cargo build --workspace`) sans warning ;
  2. `cargo clippy --workspace --all-targets -- -D warnings` passe ;
  3. `cargo fmt --all --check` passe ;
  4. `cargo test --workspace` passe ;
  5. tu me fais un résumé : ce qui marche, comment le tester à la main, ce qui reste ;
  6. **tu t'arrêtes et tu attends ma validation** avant la phase suivante.
- Ne saute jamais une phase. Ne commence pas l'interface avant que le moteur sache exporter.
- Si une décision technique importante n'est pas tranchée ici, **propose 2 options avec ta
  recommandation** et demande-moi, au lieu de choisir en silence.
- Si une bibliothèque ne marche pas comme prévu (version, API changée, compilation Windows),
  dis-le clairement, propose une alternative, ne contourne pas en cachette.
- Pas de code « TODO » laissé en plein milieu d'une fonctionnalité annoncée comme terminée.
- Vérifie les versions réelles des crates sur crates.io avant de les ajouter (ne devine pas).
- Maintiens à jour un fichier `CLAUDE.md` à la racine : architecture, commandes utiles,
  conventions, pièges rencontrés. Il sert de mémoire pour les prochaines sessions.
- Maintiens un `README.md` pour les humains : installation, compilation, utilisation.

---

## 0 bis. Git : branches, commits et push

- **Ne travaille jamais directement sur `main`.** `main` doit toujours compiler et passer les tests.
- **Une branche par fonctionnalité ou par correction**, créée depuis `main` à jour, avec un nom clair :
  - `feature/<sujet>` pour une fonctionnalité (ex. `feature/timeline-undo-redo`,
    `feature/suivi-mosse`, `feature/mcp-outil-export`) ;
  - `fix/<sujet>` pour une correction (ex. `fix/seek-decalage-audio`) ;
  - `docs/<sujet>`, `refactor/<sujet>`, `chore/<sujet>`, `test/<sujet>` pour le reste.
- **Commits fréquents, petits et cohérents** : un commit = un changement logique qui compile.
  Pas de commit fourre-tout « wip », « fix », « update ».
- **Messages au format Conventional Commits** :
  `type(portée): résumé court à l'impératif`, puis une ligne vide et un corps qui explique
  **pourquoi** (pas seulement quoi) quand ce n'est pas évident. Exemples :
  - `feat(core): ajoute le groupement de commandes pour l'undo`
  - `fix(media): corrige le décalage audio après un seek`
  - `docs(vision): explique le calcul du PSR dans docs/suivi.md`
  - `test(render): ajoute un test de snapshot pour le flou elliptique`
- **Avant chaque commit** : `cargo fmt`, `cargo clippy` et les tests concernés passent.
  Ne committe jamais de secrets, de fichiers générés lourds, ni de vidéos de test volumineuses
  (prévois un `.gitignore` propre dès la phase 0).
- **Push régulier** : pousse la branche sur le dépôt distant (GitHub) au moins à chaque
  fonctionnalité terminée et à la fin de chaque session de travail, pour ne jamais perdre de travail.
  Si aucun dépôt distant n'est configuré, demande-moi de le créer au lieu de l'ignorer.
- **Fin de fonctionnalité** : ouvre une **Pull Request** vers `main` avec une description
  claire (ce qui change, pourquoi, comment tester, captures si c'est visuel). La CI doit être verte.
  **Ne merge jamais toi-même** : c'est moi qui valide et merge.
- Mets un **tag** de version à la fin de chaque phase validée (`v0.1.0`, `v0.2.0`…) et tiens un
  `CHANGELOG.md` à jour.

---

## 0 ter. Lisibilité du code : quelqu'un qui débarque doit tout comprendre

Le code doit pouvoir être lu et compris par un développeur qui découvre le projet, sans t'avoir
sous la main. **La lisibilité passe avant l'astuce.**

### Nommage
- **Noms clairs, précis et complets**, jamais d'abréviations obscures. Le nom doit dire ce que
  la chose est ou fait, sans avoir à lire son code.
  - Fonctions : un verbe + un complément précis. ✅ `split_clip_at_time`,
    `compute_peak_to_sidelobe_ratio`, `find_clip_by_id`, `export_timeline_to_mp4`.
    ❌ `process`, `handle`, `do_it`, `calc`, `run2`.
  - Types (structs, enums, traits) : des noms qui décrivent le concept métier.
    ✅ `TimelineClip`, `TrackedBlurEffect`, `CorrelationFilterTracker`, `ExportPreset`.
    ❌ `Data`, `Manager`, `Helper`, `Thing`, `Obj`.
  - Variables : ✅ `clip_start_time`, `frame_width_in_pixels`, `tracking_confidence`,
    `remaining_frames`. ❌ `t`, `tmp`, `x2`, `val`, `res`, `d`.
  - Les variables d'une lettre ne sont acceptées que pour les indices de boucle très courtes
    ou les notations mathématiques standard, et dans ce cas commentées.
  - Booléens formulés comme une question : `is_playing`, `has_audio_track`, `should_loop`.
  - **Les unités dans le nom** quand c'est ambigu : `duration_in_seconds`, `offset_in_frames`,
    `blur_radius_in_pixels`.
- Conventions Rust respectées : `snake_case` pour fonctions et variables, `PascalCase` pour les
  types, `SCREAMING_SNAKE_CASE` pour les constantes.
- Code et identifiants **en anglais** ; documentation `.md` et commentaires en **français**
  (demande-moi si tu préfères tout en anglais pour ouvrir le projet à l'international).

### Structure
- **Fonctions courtes** qui font une seule chose (vise moins de ~40 lignes). Si une fonction
  devient longue, découpe-la en sous-fonctions bien nommées.
- **Fichiers et modules organisés par concept**, pas de fichier géant de plusieurs milliers de lignes.
- Pas de « nombres magiques » : constantes nommées et commentées
  (`const MINIMUM_PSR_FOR_CONFIDENT_TRACKING: f32 = 7.0; // seuil empirique, voir docs/suivi.md`).
- Pas de duplication : factorise dès qu'un même bloc apparaît deux fois.
- Préfère le code explicite au code « malin » : une boucle claire vaut mieux qu'une chaîne
  d'itérateurs illisible.
- Types forts plutôt que primitives nues (`ClipId`, `TimelinePosition` au lieu de `u64` partout).

### Commentaires et documentation dans le code
- **Doc-comment (`///`) sur chaque élément public** : à quoi il sert, ses paramètres, ce qu'il
  renvoie, les erreurs possibles, et un exemple quand c'est utile.
- **Doc de module (`//!`) en haut de chaque fichier** : rôle du module et comment il s'insère
  dans l'ensemble.
- Commentaires pour expliquer le **pourquoi** (choix, compromis, référence à un article ou à un
  algorithme), pas pour paraphraser le code.
- Les algorithmes (MOSSE, KCF, Lucas-Kanade, One Euro filter…) sont expliqués étape par étape
  dans le code et référencés (article original).
- `cargo doc --workspace --no-deps` doit générer une documentation propre, sans warning.

---

## 0 quater. Documentation en fichiers `.md`

En plus de `README.md`, `CLAUDE.md`, `CHANGELOG.md` et `THIRD_PARTY_LICENSES.md`, crée et tiens
à jour un dossier `docs/` :

- `docs/architecture.md` : vue d'ensemble, rôle de chaque crate, schéma des dépendances entre
  crates, parcours d'une image du fichier source jusqu'à l'export.
- `docs/modele-de-donnees.md` : projet, timeline, clips, effets, keyframes, commandes, format
  du fichier `.kadr`.
- `docs/suivi-et-floutage.md` : fonctionnement du suivi algorithmique, paramètres, limites,
  comment régler les seuils.
- `docs/mcp.md` : liste de tous les outils MCP, leurs paramètres, exemples de conversations,
  configuration de Claude Desktop et Claude Code.
- `docs/rendu-gpu.md` : pipeline wgpu, shaders, comment ajouter un effet.
- `docs/contribuer.md` : installer l'environnement, conventions de code, de nommage, de branches
  et de commits, comment lancer les tests, comment ouvrir une PR.
- `docs/decisions/` : une note courte (ADR) par décision technique importante :
  contexte, options envisagées, choix retenu, pourquoi.
- Un `README.md` court dans **chaque crate** : rôle, API principale, exemple d'utilisation.

**Règle** : une fonctionnalité n'est pas terminée tant que sa documentation `.md` n'est pas à
jour. Mets à jour les docs dans la même PR que le code.

---

## 0 quinquies. UI/UX : un logiciel beau, simple et agréable

L'interface est aussi importante que le moteur. Le but : **un débutant fait son premier montage
sans tutoriel, et un utilisateur avancé travaille vite au clavier.** Chaque écran doit être pensé
UI (apparence) **et** UX (parcours, clarté, confort).

### Méthode
- **Avant de coder l'interface (début de la phase 6)**, propose-moi des **maquettes** : parcours
  principaux (importer → monter → exporter, flouter un visage, connecter Claude) puis maquettes
  HTML statiques des écrans clés. **Attends ma validation** avant de les implémenter.
- Pour chaque fonctionnalité visible, décris d'abord le **parcours utilisateur** (ce que la
  personne veut faire, chaque étape, ce qu'elle voit) avant d'écrire le code.
- Documente le design dans `docs/design-system.md` et les parcours dans `docs/parcours-utilisateur.md`.

### Design system
- **Tokens centralisés** (variables CSS) : couleurs, espacements (échelle de 4 px), rayons,
  ombres, typographie, durées d'animation. Aucune couleur ou taille codée en dur dans un composant.
- **Bibliothèque de composants réutilisables** (boutons, champs, curseurs, menus, onglets,
  fenêtres, notifications, info-bulles) avec leurs états : normal, survol, appui, focus,
  désactivé, chargement, erreur.
- **Thème sombre par défaut** (standard du montage vidéo, l'image ressort mieux) + thème clair.
- Une police lisible (ex. Inter), une hiérarchie typographique claire, des icônes cohérentes
  d'une seule famille (ex. Lucide) avec info-bulle et raccourci clavier affiché.
- Style moderne et épuré, inspiré des meilleurs éditeurs actuels, **sans copier** l'apparence
  d'un logiciel existant.

### Principes UX
- **L'image est la star** : l'aperçu et la timeline prennent la place, les panneaux sont
  redimensionnables, repliables, et la disposition est mémorisée.
- **Simple d'abord, puissant ensuite** : les réglages courants sont visibles, les avancés sont
  dans une section « Avancé » repliée. Pas de mur de boutons.
- **Retour immédiat** : chaque action a une réponse visible en moins de 100 ms (aperçu mis à jour,
  état du bouton, animation discrète). Les tâches longues ont une barre de progression, un temps
  restant estimé et un bouton Annuler.
- **Rien n'est grave** : tout est annulable (Ctrl+Z), les suppressions importantes demandent
  confirmation ou proposent « Annuler » dans une notification, sauvegarde automatique.
- **États vides utiles** : un projet vide explique quoi faire (« Glisse tes vidéos ici ») au lieu
  d'un écran blanc.
- **Messages d'erreur humains** : ce qui s'est passé, pourquoi, et quoi faire.
  ✅ « Ce fichier n'est pas une vidéo reconnue. Formats acceptés : MP4, MOV, MKV, WebM. »
  ❌ « Error: decode failed (-1094995529) ».
- **Glisser-déposer partout où c'est naturel** : importer, placer sur la timeline, réordonner.
- **Magnétisme et guides** sur la timeline et dans l'aperçu (alignement, centre, bords), désactivables.
- **Premier lancement** : courte visite guidée facultative (3–4 étapes, passable) et projet
  d'exemple.
- **Le floutage doit être évident** : cadres clairs sur l'aperçu, plages incertaines bien visibles
  sur la timeline, correction en un clic-glisser.
- **Claude visible** : quand Claude modifie le projet, l'utilisateur le voit (notification,
  surbrillance des clips modifiés, entrée « Claude : … » dans l'historique).

### Accessibilité
- Contrastes conformes WCAG AA minimum.
- **Tout est utilisable au clavier**, focus toujours visible, ordre de tabulation logique.
- Raccourcis personnalisables et liste consultable (touche `?`).
- Taille d'interface réglable (zoom 80 %–150 %), labels ARIA sur les contrôles.
- Ne jamais transmettre une information **uniquement** par la couleur (ajouter icône ou texte).
- Respecter `prefers-reduced-motion` pour les animations.

### Mobile (phase 9)
- Interface **repensée pour le tactile**, pas une copie réduite du PC : zones tactiles de 44 px
  minimum, timeline au doigt, pincer pour zoomer, appui long pour les menus, outils principaux
  à portée du pouce en bas de l'écran.

### Contrôle qualité UI/UX à chaque PR visible
- Captures d'écran (thème sombre et clair) dans la description de la PR.
- Vérification : états vides, chargement, erreur, textes longs (traduction), petite fenêtre
  (1280×720) et grand écran.
- Navigation complète au clavier testée.
- Si une interaction te semble confuse en la testant, dis-le et propose mieux.

---

## 1. Principes non négociables

- **100 % gratuit, aucune fonctionnalité verrouillée, aucun compte obligatoire, aucune télémétrie.**
- **Tout tourne en local** : décodage, floutage, export. Les vidéos ne quittent jamais la machine.
- **Pas d'appel à l'API Claude, pas de clé API dans le logiciel.** L'intégration avec Claude se fait
  **uniquement** par un serveur MCP local que l'utilisateur branche dans Claude Desktop (ou Claude
  Code). C'est Claude, dans la conversation de l'utilisateur et avec son abonnement, qui appelle
  les outils du logiciel. Le logiciel ne « parle » jamais à Anthropic.
- **Pas d'IA ni de réseau de neurones pour le floutage.** Uniquement des algorithmes classiques
  de vision par ordinateur, déterministes et explicables (section 4). Aucun fichier de modèle
  (`.onnx`, etc.) à télécharger.
- **Licence du projet** : GPL-3.0-or-later (ou MPL-2.0 si tu as une bonne raison, demande-moi).
- **Licences des dépendances vérifiées** et listées dans `THIRD_PARTY_LICENSES.md`.
- **Le moteur est indépendant de l'interface** : les crates du cœur ne dépendent ni de Tauri,
  ni d'une bibliothèque d'interface, ni d'API spécifiques à Windows. C'est ce qui permettra l'APK.
- **Toute modification du projet passe par une commande annulable** (undo/redo), qu'elle vienne
  de la souris, du clavier ou de Claude via MCP.
- **Performances** : décodage et encodage matériels quand disponibles, rendu des effets sur GPU,
  aperçu fluide.

---

## 2. Architecture cible

Un **workspace Cargo** :

```
kadr/
├── Cargo.toml                 # workspace
├── CLAUDE.md
├── README.md
├── THIRD_PARTY_LICENSES.md
├── crates/
│   ├── kadr-core/             # modèle de données : projet, timeline, pistes, clips, effets,
│   │                          # keyframes, commandes undo/redo, sérialisation. ZÉRO I/O vidéo.
│   ├── kadr-media/            # décodage/encodage (FFmpeg), probe des fichiers, extraction audio,
│   │                          # vignettes, waveforms. Derrière des traits pour pouvoir brancher
│   │                          # MediaCodec (Android) plus tard.
│   ├── kadr-render/           # compositing des images d'une timeline à un instant t (wgpu) :
│   │                          # transformations, opacité, texte, transitions, flous.
│   ├── kadr-vision/           # algorithmes classiques : suivi de zone (floutage de visage),
│   │                          # flux optique, détection des silences dans l'audio. Pas d'IA.
│   ├── kadr-engine/           # orchestre tout : charge un projet, fournit l'aperçu,
│   │                          # lance les exports et les tâches longues (avec progression
│   │                          # et annulation). API publique unique pour les interfaces.
│   ├── kadr-tools/            # définitions des outils de montage exposés à Claude (nom,
│   │                          # description, schéma JSON des paramètres, exécution →
│   │                          # commandes kadr-core).
│   ├── kadr-mcp/              # serveur MCP local (crate `rmcp`) qui expose kadr-tools.
│   └── kadr-cli/              # binaire en ligne de commande pour tester sans interface.
├── apps/
│   ├── desktop/               # application Tauri 2 (front web + back Rust qui appelle kadr-engine)
│   └── mobile/                # (phase 9) même app Tauri 2 compilée pour Android
└── tests/fixtures/            # petites vidéos de test (générées par script FFmpeg)
```

### Choix techniques de départ

| Besoin | Choix | Notes |
|---|---|---|
| Décodage / encodage | `ffmpeg-next` (FFmpeg 7.x) | Sous Windows : FFmpeg « shared » + LLVM/clang pour bindgen, variable `FFMPEG_DIR`. Documente l'installation pas à pas dans le README. |
| Accélération matérielle | hwaccel FFmpeg : D3D11VA/NVDEC au décodage, NVENC/QSV/AMF à l'encodage | Toujours avec repli logiciel automatique. |
| Rendu GPU | `wgpu` | Shaders WGSL. Doit marcher sur Vulkan, DX12, Metal et GLES (Android). |
| Vision classique | **implémentation maison en Rust** + `image` / `imageproc` pour les primitives | Pas d'OpenCV (lourd à compiler et à embarquer sur Android). Si tu juges OpenCV indispensable, demande-moi. |
| Interface | **Tauri 2** + front **Svelte** (ou React, propose-moi) + TypeScript | Tauri 2 gère Android/iOS → même interface sur mobile. |
| MCP | `rmcp` (SDK officiel Rust) | Transport **stdio** pour Claude Desktop / Claude Code. |
| Sérialisation | `serde` + `serde_json` | Fichier projet `.kadr` = JSON versionné. |
| Erreurs | `thiserror` dans les libs, `anyhow` dans les binaires | |
| Logs | `tracing` + `tracing-subscriber` | Attention : en MCP stdio, **rien sur stdout** sauf le protocole. Logs sur stderr ou fichier. |
| Async / tâches | `tokio` côté engine/mcp, threads dédiés pour décodage | |
| Tests | unitaires + intégration sur fixtures générées + snapshots (`insta`) pour le JSON | |

Si un choix de ce tableau te semble mauvais après vérification, dis-le-moi avant de le changer.

---

## 3. Modèle de données (kadr-core)

À concevoir proprement dès la phase 1, car tout en dépend :

- **Temps** : représentation rationnelle exacte (ticks entiers d'une base de temps fixe, ou
  rationnel `num/den`), jamais de `f64` pour les positions sur la timeline.
- **Project** : paramètres (résolution, cadence, audio, couleur de fond), médias importés
  (`MediaAsset` avec chemin, hash, métadonnées), timeline.
- **Timeline** : pistes vidéo et audio ordonnées.
- **Clip** : référence à un média, `source_in`, `source_out`, `timeline_start`, vitesse,
  volume, transformation (position, échelle, rotation, recadrage), opacité, liste d'effets.
- **Effect** : enum extensible (flou de zone suivie, texte, couleur, transition…),
  chaque paramètre animable par **keyframes** (linéaire, ease, hold).
- **Effet TrackedBlur** : zone de départ (image + rectangle/ellipse), trajectoire calculée
  (une boîte par image ou keyframes interpolées), état de confiance par image, corrections
  manuelles, style (flou gaussien, pixelisation, cache plein), intensité, marge, adoucissement.
- **Commandes** : pattern Command (`apply` / `revert`), pile undo/redo, **groupes** de commandes
  (une demande de Claude qui fait 10 modifications = un seul Ctrl+Z).
- **Validation** : impossible de créer un état invalide. Erreurs explicites.
- **Versionnage du format** : `format_version` + migrations.

---

## 4. Le floutage de visage par algorithme (fonctionnalité phare)

### Contrainte
**Aucune IA, aucun réseau de neurones, aucun modèle pré-entraîné.** Le logiciel ne « reconnaît »
pas les visages : c'est l'utilisateur qui désigne la zone, et un **algorithme de suivi** la
retrouve image après image.

### Parcours utilisateur
1. L'utilisateur place la tête de lecture sur une image où le visage est bien visible.
2. Il active « Flouter une zone / un visage » et **trace un rectangle ou une ellipse** sur le visage
   (ou clique dessus : le logiciel propose alors un cadre initial par croissance de région sur la
   teinte de peau autour du clic, que l'utilisateur ajuste).
3. Une tâche d'analyse se lance (progression, annulable) : la zone est suivie **en avant et en
   arrière** sur toute la durée du clip.
4. Le résultat apparaît comme un effet sur le clip, visible dans l'aperçu.
5. Les plages où le suivi est incertain ou perdu sont **colorées sur la timeline**. L'utilisateur
   peut s'y placer, recadrer la zone, et relancer le suivi à partir de là.
6. Il choisit le style : flou gaussien, pixelisation ou cache plein ; intensité ; marge.

### Pipeline technique (kadr-vision)
Implémente, en Rust, sur des images en niveaux de gris réduites (ex. largeur 640) :

- **Suivi principal : filtre de corrélation** de type **MOSSE**, puis amélioration vers **KCF**
  (noyau gaussien, caractéristiques HOG simples) si la qualité le demande. Calculs en domaine de
  Fourier (`rustfft`). C'est rapide (des centaines d'images par seconde) et très adapté à un
  visage qui bouge.
- **Changement de taille** : estimation de l'échelle par pyramide (essayer 3 à 5 échelles autour
  de la taille courante), pour suivre un visage qui s'approche ou s'éloigne.
- **Mesure de confiance** : PSR (*peak-to-sidelobe ratio*) du pic de corrélation. En dessous d'un
  seuil → image marquée « incertaine » ; longtemps en dessous → « perdu », on arrête la mise à
  jour du modèle pour ne pas dériver.
- **Récupération** : quand la cible est perdue, recherche élargie autour de la dernière position
  par **template matching** (corrélation croisée normalisée) avec le modèle mémorisé avant la perte.
- **Appoint en mouvement rapide** : flux optique **Lucas-Kanade pyramidal** sur des points de la
  zone pour prédire le déplacement avant la corrélation.
- **Lissage** : One Euro filter (ou Kalman simple) sur la position et la taille, pour que le flou
  ne tremble pas.
- **Sécurité** : en cas de doute, **agrandir la marge** plutôt que laisser le visage apparaître.
- **Rendu** : flou appliqué sur GPU dans kadr-render, masque elliptique aux bords adoucis.
- **Cache** : trajectoire stockée dans le projet pour ne pas recalculer.
- Le même outil sert à flouter **n'importe quoi** : plaque d'immatriculation, logo, écran.

Écris des tests sur des vidéos synthétiques (un motif qui se déplace, grossit, passe derrière un
obstacle) pour mesurer l'erreur de suivi.

### Critères de réussite
- Sur une vidéo où une personne bouge normalement, le visage reste flouté sur tout le clip après
  une seule sélection, sans tremblement visible.
- Quand le suivi décroche (visage caché, sortie du cadre), la plage est signalée et la correction
  prend moins de 10 secondes.
- Analyse d'1 minute de 1080p30 en moins de 15 secondes sur un PC portable récent (indicatif,
  mesure et rapporte les chiffres réels).

---

## 5. Le pilotage depuis une conversation Claude (MCP local, sans API)

### Principe
- L'utilisateur a un abonnement Claude et utilise **Claude Desktop** (ou Claude Code).
- Il ajoute Kadr comme **serveur MCP local** dans la configuration de Claude Desktop
  (`claude_desktop_config.json`, commande = chemin vers `kadr-mcp.exe`).
- Dans sa conversation, il décrit ce qu'il veut. **Claude appelle les outils MCP de Kadr**, qui
  modifient le projet ou lancent l'export sur la machine. Aucune clé API, aucun appel réseau depuis
  Kadr.
- **Les vidéos restent sur le PC.** Une vidéo envoyée en pièce jointe dans la conversation n'est pas
  transmise au serveur MCP : l'utilisateur désigne ses fichiers par leur chemin, ou les importe dans
  Kadr, et Claude les retrouve avec `list_media` / `find_media_files`. Explique-le clairement dans le
  README et dans la description des outils.
- **Claude voit la vidéo** grâce à des outils qui renvoient des **images** (contenu image MCP) :
  une image à un instant donné, ou une planche de vignettes sur toute la durée.
- L'application desktop doit pouvoir être **ouverte en même temps** : les changements faits par
  Claude apparaissent **en direct**. Propose le mécanisme (le binaire `kadr-mcp` lancé par Claude
  Desktop se connecte à l'app ouverte par named pipe Windows / socket Unix ; si l'app est fermée,
  il travaille directement sur le fichier projet).

### Outils à exposer (au minimum)
- Fichiers : `find_media_files(dossier, filtre)` (limité aux dossiers autorisés), `import_media`,
  `probe_media`.
- Lecture : `get_project_info`, `list_media`, `get_timeline`, `get_clip`,
  `get_frame(clip_ou_media, temps)` → image, `get_contact_sheet(media, nombre)` → planche de
  vignettes, `detect_silences`.
- Montage : `create_project`, `open_project`, `add_clip`, `remove_clip`, `move_clip`, `trim_clip`,
  `split_clip`, `set_clip_speed`, `set_clip_volume`, `set_transform`, `add_track`, `remove_track`.
- Contenu : `add_text`, `edit_text`, `add_transition`, `add_color_adjustment`.
- Floutage : `blur_region(clip_id, temps, rectangle, style)` qui lance le suivi algorithmique.
  Claude repère le visage en regardant `get_frame` et fournit le rectangle en pixels.
  `get_blur_status` renvoie les plages incertaines pour que Claude puisse vérifier et corriger.
- Projet : `undo`, `redo`, `save_project`, `set_project_settings` (dont format vertical 9:16).
- Export : `export(preset, chemin)` avec presets (TikTok/Reels/Shorts 1080×1920, YouTube 1080p/4K,
  personnalisé), `get_export_status`.

### Exigences
- Descriptions d'outils rédigées pour un LLM : unités explicites (secondes, pixels), exemples,
  système de coordonnées précisé (origine en haut à gauche, pixels de la vidéo source).
- Images renvoyées à Claude redimensionnées raisonnablement (ex. 1024 px max) avec leurs
  dimensions d'origine indiquées, pour que Claude convertisse ses coordonnées correctement.
- Erreurs en clair (« le clip 12 n'existe pas, clips existants : 3, 5, 8 »).
- Un appel d'outil = un groupe d'undo, visible dans l'historique comme « Claude : … ».
- **Sécurité** : accès fichiers limité aux dossiers que l'utilisateur a autorisés dans les
  paramètres de Kadr ; jamais d'écrasement de fichier sans `overwrite: true` explicite ;
  jamais de suppression de fichiers sur le disque.
- Tâches longues (suivi, export) : l'outil répond vite avec un identifiant de tâche, Claude suit
  l'avancement avec un outil de statut.
- README : configuration pas à pas de Claude Desktop (JSON d'exemple) et de Claude Code
  (`claude mcp add`), plus 5 exemples de demandes à faire dans la conversation.

---

## 6. Plan par phases

### Phase 0 — Fondations
- Workspace Cargo, crates vides avec leur rôle documenté, CI GitHub Actions (build + clippy +
  fmt + tests sur Windows, Linux, macOS).
- Guide d'installation Windows : Rust, LLVM, FFmpeg shared, variables d'environnement.
  Script PowerShell `scripts/setup-windows.ps1` qui vérifie que tout est en place.
- Script de génération de vidéos de test avec FFmpeg (mire, compteur, bip audio, motif mobile).
- `CLAUDE.md`, `README.md`, licence.
- ✅ Fini quand : `cargo build` et `cargo test` passent en local et en CI.

### Phase 1 — Modèle de données
- kadr-core complet (section 3) : types, commandes, undo/redo, groupes, validation,
  sérialisation, migrations. Tests exhaustifs.
- ✅ Fini quand : on peut construire une timeline en code, la sauver, la recharger, et
  annuler/refaire toutes les opérations.

### Phase 2 — Média et export en ligne de commande
- kadr-media : probe, décodage vidéo + audio, seek précis à l'image, encodage H.264/AAC en MP4,
  accélération matérielle avec repli.
- kadr-render (version CPU simple au début) + kadr-engine : rendu d'une timeline.
- kadr-cli : `kadr-cli render projet.kadr sortie.mp4`, `kadr-cli probe fichier.mp4`.
- ✅ Fini quand : un projet avec 3 clips coupés, une piste audio et un changement de vitesse
  s'exporte correctement, son synchronisé (vérifié par test automatique).

### Phase 3 — Rendu GPU
- Compositing sur wgpu : transformations, opacité, recadrage, fondus, texte (`cosmic-text` ou
  `glyphon`), flou gaussien et pixelisation sur zone masquée.
- Benchmarks (`criterion`) en 1080p et 4K.
- ✅ Fini quand : export identique visuellement à la phase 2 mais nettement plus rapide
  (chiffres rapportés).

### Phase 4 — Floutage par suivi algorithmique (moteur)
- kadr-vision : MOSSE → KCF, échelle, PSR, récupération, Lucas-Kanade, lissage (section 4).
- CLI : `kadr-cli blur video.mp4 --at 2.5 --rect 420,180,160,200 -o sortie.mp4`
  et `--debug` qui exporte une vidéo avec le cadre suivi dessiné et la confiance affichée.
- ✅ Fini quand : critères de la section 4 atteints, chiffres de performance rapportés.

### Phase 5 — Serveur MCP (mode fichier)
- kadr-tools + kadr-mcp en stdio, travaillant directement sur un fichier projet.
- Test réel : je l'ajoute à Claude Desktop, je demande un montage simple dans une conversation,
  le projet est modifié et s'exporte.
- ✅ Fini quand : tous les outils de la section 5 marchent, avec tests d'intégration qui
  simulent des appels MCP (y compris les outils qui renvoient des images).

### Phase 6 — Application desktop (MVP)
- Tauri 2 : bibliothèque de médias (glisser-déposer), aperçu fluide, timeline multipiste (zoom,
  magnétisme, vignettes, forme d'onde), inspecteur, raccourcis (espace, J/K/L, S pour couper,
  Ctrl+Z/Ctrl+Y, Suppr), export avec presets et progression.
- Interface sobre et moderne, thème sombre par défaut, **français et anglais** dès le départ.
- ✅ Fini quand : je peux faire un vrai montage court de A à Z à la souris et l'exporter.

### Phase 7 — Floutage et Claude dans l'interface
- Outil de sélection de zone dans l'aperçu, suivi avec progression, plages incertaines sur la
  timeline, correction manuelle et relance du suivi.
- Liaison `kadr-mcp` ↔ application ouverte : les modifications de Claude apparaissent en direct,
  avec une notification « Claude a modifié le projet ».
- Écran « Connecter à Claude » dans les paramètres : génère la config Claude Desktop en un clic
  et gère les dossiers autorisés.
- ✅ Fini quand : dans Claude Desktop je dis « floute le visage de la personne à gauche dans
  interview.mp4 et exporte pour TikTok » et ça marche (Claude regarde l'image, donne le rectangle,
  vérifie le résultat).

### Phase 8 — Fonctionnalités complètes
Toutes **gratuites**, par ordre de priorité (demande-moi avant de commencer chacune) :
1. Suppression automatique des silences (analyse du volume, algorithmique)
2. Transitions (fondu, glissé, zoom) et animations de texte
3. Étalonnage (exposition, contraste, saturation, température) + LUT .cube
4. Stabilisation (flux optique + lissage de trajectoire)
5. Keyframes éditables dans l'interface (courbes)
6. Proxies automatiques pour les vidéos 4K lourdes
7. Sous-titres : import/édition SRT (la transcription automatique demanderait de l'IA, on en
   reparlera)

### Phase 9 — APK Android
- Vérifier que kadr-core, kadr-render, kadr-vision, kadr-engine compilent pour
  `aarch64-linux-android` (cargo-ndk).
- Décodage/encodage matériel via MediaCodec derrière les traits de kadr-media ; wgpu en Vulkan/GLES.
- App Tauri 2 Android : interface tactile (timeline au doigt, pincer pour zoomer), galerie,
  export vers la galerie, permissions.
- **Claude sur mobile** : un serveur MCP local ne peut pas être branché à l'app Claude du téléphone.
  L'APK fonctionne donc sans Claude dans un premier temps. Plus tard, étudie et présente-moi les
  options (ex. le téléphone se connecte au Kadr du PC, ou connecteur MCP distant), sans API payante.
- ✅ Fini quand : une APK debug s'installe sur mon téléphone, permet un montage simple, le
  floutage par suivi et l'export.

### Phase 10 — iOS (plus tard, nécessite un Mac)

---

## 7. Qualité et performance

- Aucun `unwrap()`/`expect()` dans les libs hors tests (sauf invariant prouvé et commenté).
- Pas de `unsafe` sauf nécessité (FFI), toujours commenté.
- Tâches longues : progression, annulation, jamais bloquer l'interface.
- Pool de buffers pour les images, pas d'allocation par image dans les boucles chaudes.
- Mesure avant d'optimiser : benchmarks `criterion` sur décodage, rendu, suivi.
- Logs utiles (`tracing`), fichier de log consultable depuis l'app.
- Relinker les médias déplacés, sauvegarde automatique et récupération après crash.

---

## 8. Pour commencer

1. Lis entièrement ce document.
2. Pose-moi les questions qui bloqueraient vraiment la phase 0 ou la phase 1
   (pas plus de 5), avec ta recommandation pour chacune.
3. Vérifie mon environnement (Rust, LLVM, FFmpeg) et dis-moi ce qui manque, avec les commandes
   PowerShell pour l'installer.
4. Démarre la **phase 0**, puis arrête-toi pour validation.
