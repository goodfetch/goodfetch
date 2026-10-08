# Empreintes TLS et HTTP/2 du 8 octobre 2026

Contrat de l'ADR-002 sur la cohérence d'identité, exécuté en `local-dev` sur sources commitées. Run retenu : `tls-fingerprint-2026-10-08T17-42-02-124Z-1dec60`. 8 clients, 3 blocs. Un relais TCP lit le ClientHello de chaque connexion et calcule son JA4 ; le serveur HTTP/2 relève les réglages, l'ordre des pseudo-en-têtes et l'ordre des en-têtes. Le contrat : un client garde la même identité TLS et HTTP/2 pendant toute une session.

| Client | JA4 | Comme Chrome (version, compte, chiffrements) | Réglages HTTP/2 de Chrome | Sessions cohérentes |
| --- | --- | --- | --- | --- |
| node-cdp | `t13d1518h2_8daaf6152771_007055c22503 …` | oui | oui | 3/3 |
| rust-cdp | `t13d1518h2_8daaf6152771_3afe5229d395 …` | oui | oui | 3/3 |
| impit-http | `t13d1516h2_8daaf6152771_afc95f72a295` | chiffrements identiques | non | 3/3 |
| crawlee-http | `t13d1513h2_8daaf6152771_ff9cead5a15b …` | chiffrements identiques | non | 2/3 |
| wreq-http | `t13d2811h2_257f3020b3a2_78e6aca7449b` | non | non | 3/3 |
| reqwest-http | `t13d1011h2_61a7ad8aa9b6_3a8073edd8ef` | non | non | 3/3 |
| spider-http | `t13d1011h2_61a7ad8aa9b6_3a8073edd8ef` | non | non | 3/3 |
| node-http | `t13d5212h1_b262b3658495_8e6e362c5eac` | non | — | 3/3 |
- **Chrome** varie d'un lancement à l'autre, mais pas pendant une session : la partie « extensions » du JA4 change à nombre d'extensions égal. Ce sont les expériences de Chrome (field trials) qui font tourner certaines extensions. Un client qui imite Chrome doit suivre cette distribution, pas une empreinte figée.
- **Impit**, en émulation Chrome, présente les mêmes chiffrements que Chrome, mais deux extensions de moins, et une taille de table d'en-têtes HTTP/2 différente : son profil est antérieur à ce Chrome.
- **Crawlee** (got-scraping) imite les chiffrements de Chrome, mais **change d'empreinte TLS au milieu d'une session** dans un bloc sur trois. C'est l'incohérence que l'ADR-002 interdit.
- **wreq** sans émulation configurée présente une empreinte BoringSSL générique. reqwest et Spider partagent la même empreinte rustls. node-http (fetch) reste en HTTP/1.1.
