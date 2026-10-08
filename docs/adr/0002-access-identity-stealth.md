# ADR-002 : accès aux sites, identité des agents et options stealth

**Statut : principe décidé par le porteur du projet le 8 octobre 2026 ; modalités proposées, à adopter explicitement lors de l'implémentation.**
**Date : 8 octobre 2026.**
**Décision finale : utilisateur / porteur du projet.**

## Contexte

Proxies, empreintes navigateur cohérentes et options stealth font partie de l'offre standard des services de collecte et de navigateurs hébergés. Le dossier les prévoit déjà : configuration proxy/fingerprint cohérente ([PRD, story 34](../../issues/prd.md:62)), pool de proxies avec sticky sessions, santé, cooldown et rotation ([architecture §4.1](../architecture.md:144)), transports spécialisés en fingerprinting avec profil cohérent ([architecture §5.1](../architecture.md:177)).

En parallèle, l'accès identifié devient une voie d'accès reconnue :

- Cloudflare définit un bot ou agent **Verified** par une identification déterministe — signature Web Bot Auth, liste d'IP publiée ou reverse DNS — et un comportement non abusif, dont le respect de `robots.txt` et des directives de crawl. Depuis le 1er juillet 2026, les agents signés y sont classés **Direct** ou **Intermediary** ; un service peut être retiré de l'allowlist si son trafic ne correspond pas à l'usage déclaré. [Verified bots, mis à jour le 1er juillet 2026](https://developers.cloudflare.com/bots/concepts/bot/verified-bots/).
- Web Bot Auth est un document du groupe de travail IETF depuis le 1er septembre 2026 (auteurs Cloudflare et Google) : signatures HTTP RFC 9421, en-tête `Signature-Agent`, annuaire de clés JWKS sur une URI well-known. Le texte reste un Internet-Draft. [draft-ietf-webbotauth-httpsig-protocol-00](https://datatracker.ietf.org/doc/draft-ietf-webbotauth-httpsig-protocol/).
- Browserbase propose à la fois ses options d'accès navigateur et une identité signée en bêta, et présente désormais ce travail comme une **continuité d'identité** entre sessions plutôt qu'une évasion ponctuelle. [Web Bot Auth chez Browserbase](https://docs.browserbase.com/platform/identity/web-bot-auth), [billet du 6 avril 2026](https://www.browserbase.com/blog/identity).

Deux éléments juridiques modifient le risque de certaines options :

- **Reddit v. SerpApi (SDNY, 31 juillet 2026)** : au stade de la motion to dismiss, le tribunal considère que CAPTCHA et détection de bots peuvent constituer des mesures techniques de contrôle d'accès au sens du DMCA §1201, même si le contenu est visible par des humains. Les demandes de contournement (§1201(a)(1)(A)) et de *trafficking* (§1201(a)(2)) survivent contre le fournisseur d'un outil de scraping décrit comme utilisant proxies et techniques imitant le comportement humain. Ce n'est pas un jugement au fond. [Synthèse Loeb & Loeb](https://www.loeb.com/en/insights/publications/2026/08/reddit-v-serpapi-llc).
- **CNIL, fiche moissonnage** : pour une collecte de **données personnelles destinée à constituer des bases d'entraînement d'IA sur la base de l'intérêt légitime**, exclure les sites qui s'y opposent par `robots.txt` ou CAPTCHA fait partie des mesures obligatoires. Le périmètre est cette finalité, pas tout scraping. [Fiche du 19 juin 2025](https://www.cnil.fr/fr/focus-interet-legitime-collecte-par-moissonnage), [version anglaise du 5 janvier 2026](https://www.cnil.fr/en/legal-basis-legitimate-interest-focus-sheet-measures-implement-case-data-collection-web-scraping).

Ces éléments ne sont pas un avis juridique. La question tranchée ici n'est donc pas *s'il faut* des options stealth, mais **comment les livrer** : profils, défauts, répartition open source/cloud, modalité CAPTCHA, garde-fous et conséquences sur le modèle de session.

## Décision

### 1. Trois profils d'accès, exclusifs pour une session

| Profil | Contenu | Défaut |
| --- | --- | --- |
| `identified` | Agent déclaré : User-Agent avec jeton produit et URL de contact, signature Web Bot Auth, politique de préférences des sites appliquée | **Oui** |
| `standard` | Navigation et HTTP génériques, sans signature ni masquage ; politique de préférences appliquée | Non |
| `stealth` | Toutes les options du catalogue ci-dessous, activées individuellement | Non, opt-in explicite |

Une session possède un seul profil, fixé à son ouverture. Signer une requête puis masquer l'automatisation dans la même session est contradictoire : cela expose la clé et la réputation de l'identité signée, et contredit l'usage déclaré exigé par les programmes de vérification. Changer de profil est une transition explicite qui ouvre une nouvelle session, comme le changement de backend ([architecture §5.2](../architecture.md:183)).

### 2. Défaut conforme (`identified`)

- Signature Web Bot Auth avec clés propres à l'installation en local, **clés propres à chaque tenant** dans le cloud ; annuaire de clés publié, rotation et révocation.
- Enregistrement du service cloud comme agent vérifié (Cloudflare Verified *Intermediary*, puis autres fournisseurs d'anti-bot qui vérifient Web Bot Auth).
- Politique avant chaque requête : `robots.txt` (RFC 9309) dont crawl-delay, Content-Signal, TDMRep, meta `noai`, puis vocabulaire IETF aipref lorsqu'il sera stabilisé.
- Finalité déclarée par workflow (search, ai-input, ai-train, agent, data-collection, transact), alignée sur les catégories de comportement Cloudflare.
- Politesse et débit par hôte, adaptés aux réponses `429`/`Retry-After`.
- `401`, `403`, `402` et pages de challenge sont des **signaux d'arrêt ou de licence**, jamais un succès.

### 3. Catalogue `stealth` : toutes les options

| Famille | Options | Open source | Cloud |
| --- | --- | --- | --- |
| Proxies | Datacenter, résidentiels, mobiles, ISP ; BYO fournisseur ; sticky sessions, géolocalisation, rotation raisonnée, santé, cooldown, coût par Go mesuré | Gestion du pool et adapters fournisseurs (BYO) | Pools managés, isolés par tenant |
| Empreinte réseau | Profils TLS et HTTP/2 cohérents avec le navigateur annoncé, ordre et casse des en-têtes, client hints | Transports d'impersonation (classe wreq/Impit) et profils de base | Catalogue de profils maintenu et daté |
| Empreinte navigateur | Profils cohérents : User-Agent, plateforme, écran, polices, WebGL/canvas, langue, fuseau et géolocalisation alignés sur la sortie proxy ; profils persistants ; mode headful | Moteur de cohérence et profils de base | Profils maintenus, navigateurs patchés maintenus |
| Signaux d'automatisation | Réduction des marqueurs d'automatisation du pilote et du navigateur ; navigateurs patchés en plugin (classe Patchright/Camoufox) | Interfaces et plugins | Builds maintenus |
| Comportement | Rythmes de saisie et trajectoires de pointeur réalistes, en option par action | Oui | Oui |
| Continuité | Réchauffement de session, persistance cookies/stockage, même identité au retour | Oui | Oui |
| CAPTCHA | Voir §4 | Détection, handoff, interface de solveur | Résolution managée |
| Détection de blocage | Classifieur de pages de challenge, interstitiels et `200` trompeurs ; escalade de profil selon politique | Oui | Statistiques de réussite par site et profil |

Chaque option est déclarée dans le plan ou la politique du workflow, visible dans le journal et comptée dans le budget de tentatives. Rotation de proxy, de session et réparation consomment le même budget total ([architecture §3.4](../architecture.md:125)).

### 4. CAPTCHA : toutes les modalités, livrées par niveaux

1. **Détection** systématique, avec erreur typée ; jamais de faux succès.
2. **Handoff humain** dans la session : l'opérateur résout, le run reprend. Open source.
3. **Adapter de solveur tiers** : interface ouverte, compte et clé fournis par le client. Open source.
4. **Résolution managée** dans le cloud : offre contractuelle, workspace vérifié.

Recommandation : livrer 1 à 3 avec le framework et ouvrir le niveau 4 après revue juridique, en particulier pour l'exposition aux États-Unis. Le risque de *trafficking* évoqué dans Reddit v. SerpApi vise le fournisseur de l'outil ; il porte donc sur Automan en tant qu'éditeur, pas seulement sur ses clients.

### 5. Garde-fous

- **Politique de workspace** : un administrateur peut autoriser ou interdire `stealth` et chaque famille d'options.
- **Provenance** : chaque tentative et chaque enregistrement portent le profil d'accès, l'identité de signature éventuelle et l'état des préférences du site au moment de la requête (hash de `robots.txt`/signaux).
- **Cloud** : vérification des clients et politique d'usage acceptable pour `stealth` ; pools d'IP et réputation isolés par tenant ; kill switch par tenant ; journaux d'audit exportables.
- **Données personnelles** : un workflow déclarant des données personnelles et une finalité `ai-train` affiche l'obligation CNIL rappelée ci-dessus ; activer `stealth` sur ce couple exige une confirmation explicite, journalisée.
- **Self-host** : refus par défaut des redirections vers des adresses privées (protection SSRF), avec allowlist explicite pour les tests ; le lab a observé ce comportement natif chez Spider.

## Répartition open source / cloud

Le framework Apache-2.0 contient tous les mécanismes : profils, cohérence d'identité, gestion des proxies BYO, transports d'impersonation, détection de blocage, handoff et interface de solveur, signature Web Bot Auth avec clés autogérées, moteur de politique. Il fonctionne sans clé cloud.

Le cloud vend ce qui exige une **maintenance continue** : pools de proxies managés, catalogue de profils réseau et navigateur tenu à jour, navigateurs patchés maintenus, résolution CAPTCHA managée, annuaire de clés hébergé et enregistrements auprès des vérificateurs, isolation de réputation et statistiques de réussite par site. Les profils publiés dans un dépôt public sont rapidement identifiables par les éditeurs d'anti-bot ; la valeur vendue est leur mise à jour, pas leur existence.

## Conséquences architecturales

- La `Session` porte un **profil d'identité complet** : signature ou absence de signature, sortie proxy, profil réseau, profil navigateur, cookies et stockage. Ces éléments sont cohérents entre eux et avec la géolocalisation du proxy.
- Ce profil est **conservé lorsqu'une requête passe de HTTP au navigateur** dans la même session. Une escalade de moteur qui change d'empreinte crée elle-même le signal de bot.
- Les erreurs de [l'API](../api.md) s'enrichissent de codes distincts pour un challenge détecté, un refus par politique du site et une demande de paiement (`402`). Le terme exact reste à fixer dans la revue d'API.
- Les profils réseau et navigateur sont des artefacts versionnés et datés, avec la version du navigateur qu'ils imitent ; un profil périmé est signalé.
- Le journal enregistre pour chaque tentative : profil, options actives, décision de politique et résultat de la détection de blocage.

## Options considérées

| Option | Avantages | Limites | Statut |
| --- | --- | --- | --- |
| Stealth seul, comme la plupart des services | Taux d'accès maximal sur sites protégés | Aucune voie d'accès vérifiée pour les clients entreprise ; réputation partagée exposée | Écartée |
| Accès identifié seul | Risque juridique et réputationnel minimal | Taux de réussite insuffisant sur sites protégés ; écarté par le porteur | Écartée |
| **Profils exclusifs, `identified` par défaut, `stealth` complet en opt-in** | Couvre le marché et l'accès vérifié ; contrôle explicite et traçable | Deux familles à maintenir ; règles de transition de session | **Retenue** |

## Risques

- **Réputation partagée** : un tenant abusif peut dégrader l'identité d'intermédiaire ou les IP partagées. Réponse : clés et pools par tenant, vérification des clients, kill switch.
- **Juridique** : CAPTCHA managé et outillage de contournement vendus par l'éditeur (§1201 aux États-Unis) ; obligations RGPD des clients. Réponse : niveaux CAPTCHA, revue juridique avant le niveau 4, provenance, confirmation explicite sur données personnelles.
- **Course permanente** : les profils vieillissent. Réponse : profils datés, mesures de réussite par site et profil, maintenance financée par le cloud.
- **Identité contradictoire** : mélanger signature et masquage. Réponse : profils exclusifs par session.

## Actions de qualification

- [ ] **Cohérence d'identité** : fixture locale qui enregistre empreinte TLS (JA4), paramètres HTTP/2, ordre des en-têtes, User-Agent et client hints, puis vérifie leur stabilité lorsqu'une même session passe de HTTP au navigateur.
- [ ] **Web Bot Auth** : fixture qui vérifie la signature RFC 9421, l'annuaire de clés, `created`/`expires`, la rotation et le refus d'une clé révoquée.
- [ ] **Politique** : fixtures `robots.txt`, Content-Signal et TDMRep ; refus journalisé ; une page de challenge servie en `200` ne doit jamais produire un résultat vérifié.
- [ ] **Proxies** : proxy local de test pour sticky sessions, rotation, santé, cooldown et budget de tentatives.
- [ ] **Transitions** : changement de profil refusé dans une session active, nouvelle session explicite acceptée.
- [ ] **Live** : campagne séparée, hors CI, sur sites autorisés ; ses résultats ne sont jamais agrégés aux scores hermétiques du lab.

Les gates de promotion restent celles de la [roadmap](../roadmap-validation.md) : comportement conforme, limites visibles, aucun succès annoncé pour un effet ou un accès non vérifié.
