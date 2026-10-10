# Empreintes TLS et HTTP/2 du 8 octobre 2026

Contrat de l'ADR-002 sur la cohérence d'identité, exécuté en `local-dev` sur sources commitées. Run retenu : `tls-fingerprint-2026-10-08T19-32-26-736Z-f42991`, après correction du calcul JA4 (voir la fin du rapport). 8 clients, 3 blocs. Un relais TCP lit le ClientHello de chaque connexion et calcule son JA4 ; le serveur HTTP/2 relève les réglages, l'ordre des pseudo-en-têtes et l'ordre des en-têtes. Le contrat : un client garde la même identité TLS et HTTP/2 pendant toute une session.

| Client | JA4 | Comme Chrome (version, compte, chiffrements) | Réglages HTTP/2 de Chrome | Sessions cohérentes |
| --- | --- | --- | --- | --- |
| node-cdp | `t13d1518h2_8daaf6152771_4980c97edce0` | oui | oui | 3/3 |
| rust-cdp | `t13d1518h2_8daaf6152771_4980c97edce0` | oui | oui | 3/3 |
| impit-http | `t13d1516h2_8daaf6152771_afc95f72a295` | chiffrements identiques | non | 3/3 |
| crawlee-http | `t13d1513h2_8daaf6152771_ff9cead5a15b` puis `t13d2313h2_24fc43eb1c96_ff9cead5a15b` | chiffrements identiques, puis une autre liste | non | 2/3 |
| wreq-http | `t13d2811h2_257f3020b3a2_78e6aca7449b` | non | non | 3/3 |
| reqwest-http | `t13d1011h2_61a7ad8aa9b6_3a8073edd8ef` | non | non | 3/3 |
| spider-http | `t13d1011h2_61a7ad8aa9b6_3a8073edd8ef` | non | non | 3/3 |
| node-http | `t13d5212h1_b262b3658495_8e6e362c5eac` | non | — | 3/3 |
- **Chrome** présente un seul JA4, le même pour les deux pilotes CDP, d'une connexion, d'un contexte et d'un lancement à l'autre. L'ordre de ses 18 extensions est tiré au sort à chaque connexion, ce que JA4 neutralise en les triant.
- **Impit**, en émulation Chrome, présente les mêmes chiffrements que Chrome, mais deux extensions de moins, et une taille de table d'en-têtes HTTP/2 différente : son profil est antérieur à ce Chrome.
- **Crawlee** (got-scraping) imite les chiffrements de Chrome, mais **change d'empreinte TLS au milieu d'une session** dans un bloc sur trois : il passe de 15 à 23 suites de chiffrement. C'est l'incohérence que l'ADR-002 interdit.
- **wreq** sans émulation configurée présente une empreinte BoringSSL générique. reqwest et Spider partagent la même empreinte rustls. node-http (fetch) reste en HTTP/1.1.

## Correction du 8 octobre au soir

La première version de ce rapport s'appuyait sur `tls-fingerprint-2026-10-08T17-42-02-124Z-1dec60`. Elle concluait que Chrome change de JA4 d'un lancement à l'autre sous l'effet de ses expériences. C'était un défaut du calcul :
- Chrome place en tête de ses algorithmes de signature une valeur GREASE, tirée à nouveau pour chaque connexion ;
- le JA4 du lab l'ignorait dans les chiffrements et les extensions, mais pas dans les algorithmes de signature.

Le défaut est corrigé (`fixtures/tls-fingerprint.ts`, avec un test). Le run précédent est classé `superseded`. Le constat sur Crawlee tient : la liste de chiffrements elle-même change.

Le relais calcule aussi une empreinte de base sans `pre_shared_key`. Une poignée de main reprise (session TLS réutilisée) ajoute en effet cette extension, et elle est comparée à la première poignée de main du même client.

