# Reference documents

The standards this system implements, kept verbatim so that a vector can
be checked against its source without a network, and so that the source
cannot change under a test that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement.

This directory holds what the RFC Editor publishes.
[`docs/oasis/`](../oasis/README.md) holds what OASIS publishes, under the
same rule and for the same reason (D-100).

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `rfc791.txt` | RFC 791, *Internet Protocol*, J. Postel, September 1981 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc791.txt` | 94892 | `6cfb387fcecfc1b72f2f69343c5b6951b5d263d708162e2b5c66ab8f394f6265` |
| `rfc792.txt` | RFC 792, *Internet Control Message Protocol*, J. Postel, September 1981 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc792.txt` | 29186 | `58714393ded142bacf188d7e8977eef98f4110c4c87ac94595f750df5664c2c6` |
| `rfc826.txt` | RFC 826, *An Ethernet Address Resolution Protocol*, D. C. Plummer, November 1982 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc826.txt` | 21556 | `01bc62fe6a37e90f1246ac43e8e145f1322b4ed1474836145c3da93d2bd3c8a6` |
| `rfc894.txt` | RFC 894, *A Standard for the Transmission of IP Datagrams over Ethernet Networks*, C. Hornig, April 1984 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc894.txt` | 5697 | `be88b9301e53f986aca3a0e55e488d1d79bae3f88fe3f257640397bc089e7035` |
| `rfc1035.txt` | RFC 1035, *Domain Names — Implementation and Specification*, P. Mockapetris, November 1987 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc1035.txt` | 122549 | `d14ae809fc9b41bbcae26bb38c937c9515808b944f3252b00de9fe0e95f4fdfb` |
| `rfc1071.txt` | RFC 1071, *Computing the Internet Checksum*, R. Braden, D. Borman, C. Partridge, September 1988 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc1071.txt` | 53524 | `e10dfd6816447843d47a7f1b990eba756a791a6308fd5b698a6276075a8e4f9b` |
| `rfc1122.txt` | RFC 1122, *Requirements for Internet Hosts — Communication Layers*, R. Braden (ed.), October 1989 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc1122.txt` | 289148 | `9f526e6bebc868324fedb90aebbcf6e5b15c53fd373ca5d5ce1c2cdcd264e04f` |
| `rfc1950.txt` | RFC 1950, *ZLIB Compressed Data Format Specification version 3.3*, P. Deutsch, J-L. Gailly, May 1996 | 2026-09-07 from `https://www.rfc-editor.org/rfc/rfc1950.txt` | 20502 | `8f0475a5c984657bf26277f73df9456c9b97f175084f0c1748f1eb1f0b9b10b9` |
| `rfc1951.txt` | RFC 1951, *DEFLATE Compressed Data Format Specification version 1.3*, P. Deutsch, May 1996 | 2026-09-07 from `https://www.rfc-editor.org/rfc/rfc1951.txt` | 36944 | `5ebf4b5b7fe1c3a0c0ab9aa3ac8c0f3853a7dc484905e76e03b0b0f301350009` |
| `rfc2104.txt` | RFC 2104, *HMAC: Keyed-Hashing for Message Authentication*, H. Krawczyk, M. Bellare, R. Canetti, February 1997 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc2104.txt` | 22297 | `64d5245a9101929025336e470e3737f118704001249d503c85a86e19fe9fbb01` |
| `rfc2131.txt` | RFC 2131, *Dynamic Host Configuration Protocol*, R. Droms, March 1997 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc2131.txt` | 113738 | `a043b705785b81762505ded4cf71d61392b2b1da2b4ce1e32c13bc984de5f4a5` |
| `rfc2132.txt` | RFC 2132, *DHCP Options and BOOTP Vendor Extensions*, S. Alexander, R. Droms, March 1997 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc2132.txt` | 63670 | `0cfbedab7cfe859624ae78c07a722bc51bf8ea123a7b43b099fc546ddcf00b90` |
| `rfc2313.txt` | RFC 2313, *PKCS #1: RSA Encryption Version 1.5*, B. Kaliski, March 1998 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc2313.txt` | 37777 | `2d93e9f0f02343a29a8f64ad507e1779f65377319ed4945b6ac3dcb1f74fd69c` |
| `rfc2464.txt` | RFC 2464, *Transmission of IPv6 Packets over Ethernet Networks*, M. Crawford, December 1998 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc2464.txt` | 12725 | `f9554329ef1f4e093513e5b6f7af00bb5d206710e0c61d08bd578fa5f00aee9a` |
| `rfc3279.txt` | RFC 3279, *Algorithms and Identifiers for the Internet X.509 Public Key Infrastructure Certificate and Certificate Revocation List (CRL) Profile*, W. Polk, R. Housley, L. Bassham, April 2002 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc3279.txt` | 53833 | `6d3f19f18e17fa1c68da5aaf4021327748fabca840d7300443b77357a1fc1614` |
| `rfc3526.txt` | RFC 3526, *More Modular Exponential (MODP) Diffie-Hellman groups for Internet Key Exchange (IKE)*, T. Kivinen, M. Kojo, May 2003 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc3526.txt` | 19166 | `6c95b231eb1c2d9ffa480efbe3ae53477231271f3438570a3af370b2aa4a968b` |
| `rfc3596.txt` | RFC 3596, *DNS Extensions to Support IP Version 6*, S. Thomson, C. Huitema, V. Ksinant, M. Souissi, October 2003 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc3596.txt` | 14093 | `2a3d44744086f66a7ba00e7d282dc1f740a3e0b63b4b6f7c763ffc0e61ee1dbb` |
| `rfc4055.txt` | RFC 4055, *Additional Algorithms and Identifiers for RSA Cryptography for use in the Internet X.509 Public Key Infrastructure Certificate and Certificate Revocation List (CRL) Profile*, J. Schaad, B. Kaliski, R. Housley, June 2005 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc4055.txt` | 57479 | `b8a1ef3fb135c32aed4eee121264f3ec83a46def746c8fe68e05bd1b60324e9a` |
| `rfc4231.txt` | RFC 4231, *Identifiers and Test Vectors for HMAC-SHA-224, HMAC-SHA-256, HMAC-SHA-384, and HMAC-SHA-512*, M. Nystrom, December 2005 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc4231.txt` | 17725 | `72178527ce93500e730bc8eb182b857e583096d652b64ece0879c52ba1df973b` |
| `rfc4250.txt` | RFC 4250, *The Secure Shell (SSH) Protocol Assigned Numbers*, S. Lehtinen, C. Lonvick, Ed., January 2006 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc4250.txt` | 44010 | `eb3a9d7c3e1fae53c038d5ae62a73e62280d1dde8b246129969c95e4d0909341` |
| `rfc4251.txt` | RFC 4251, *The Secure Shell (SSH) Protocol Architecture*, T. Ylonen, C. Lonvick, Ed., January 2006 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc4251.txt` | 71750 | `23a5052796d29edc2f0ff4f5be557df244544d129cfc26ed51971a6485eff4ef` |
| `rfc4252.txt` | RFC 4252, *The Secure Shell (SSH) Authentication Protocol*, T. Ylonen, C. Lonvick, Ed., January 2006 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc4252.txt` | 34268 | `a36b882cf77b107335d49921b5e955b4ba78e253205dfbce5096ebc0e78bdf14` |
| `rfc4253.txt` | RFC 4253, *The Secure Shell (SSH) Transport Layer Protocol*, T. Ylonen, C. Lonvick, Ed., January 2006 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc4253.txt` | 68263 | `b471173305154f6c66facffcf7a4a25fc5c32765cb30542120be6c20a88f1ed3` |
| `rfc4254.txt` | RFC 4254, *The Secure Shell (SSH) Connection Protocol*, T. Ylonen, C. Lonvick, Ed., January 2006 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc4254.txt` | 50338 | `bfc727e9350067eaa49edeecc3d32f1e3518dd57cf3c36ceed02177156c5703f` |
| `rfc4291.txt` | RFC 4291, *IP Version 6 Addressing Architecture*, R. Hinden, S. Deering, February 2006 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc4291.txt` | 52897 | `4d58dff6b432d5d524bf3a3b7f0337a4177fa65f92ed72f2a92e97b471de48b2` |
| `rfc4443.txt` | RFC 4443, *Internet Control Message Protocol (ICMPv6) for the Internet Protocol Version 6 (IPv6) Specification*, A. Conta, S. Deering, M. Gupta (ed.), March 2006 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc4443.txt` | 48969 | `f20d1de8878e1142000bfeaa3ad1b0540fab7653a395cf7b07e81d2da2f650bf` |
| `rfc4861.txt` | RFC 4861, *Neighbor Discovery for IP version 6 (IPv6)*, T. Narten, E. Nordmark, W. Simpson, H. Soliman, September 2007 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc4861.txt` | 235106 | `1a4309c117d765a7c0edfcb2297fc90c3c09f0fcae255f15e53b20748bcc09fb` |
| `rfc4862.txt` | RFC 4862, *IPv6 Stateless Address Autoconfiguration*, S. Thomson, T. Narten, T. Jinmei, September 2007 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc4862.txt` | 72482 | `6d3d2af5d2f6c9109b4ebd038ff7a6cec56771bbb0b5d8bd520357e03f097aa2` |
| `rfc5480.txt` | RFC 5480, *Elliptic Curve Cryptography Subject Public Key Information*, S. Turner, D. Brown, K. Yiu, R. Housley, T. Polk, March 2009 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5480.txt` | 36209 | `593bf29fd0da2ff8b903c3ebf1c9d189a770039159e2ba46a0c3b91355037f26` |
| `rfc5656.txt` | RFC 5656, *Elliptic Curve Algorithm Integration in the Secure Shell Transport Layer*, D. Stebila, J. Green, December 2009 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc5656.txt` | 44259 | `0cc84636afaf4860f97271b5b411be0393f71544d3c219f25df2d78201a4949a` |
| `rfc5756.txt` | RFC 5756, *Updates for RSAES-OAEP and RSASSA-PSS Algorithm Parameters*, S. Turner, D. Brown, K. Yiu, R. Housley, T. Polk, January 2010 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc5756.txt` | 12017 | `304a0b826bf211028209db48bd1419c1088127f33e1689d62028378c739e67f5` |
| `rfc5758.txt` | RFC 5758, *Internet X.509 Public Key Infrastructure: Additional Algorithms and Identifiers for DSA and ECDSA*, Q. Dang, S. Santesson, K. Moriarty, D. Brown, T. Polk, January 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5758.txt` | 15834 | `4d02628ff0875a1960d34be584a68f88528b96242bdc5a05a40a29ef01cf1532` |
| `rfc5869.txt` | RFC 5869, *HMAC-based Extract-and-Expand Key Derivation Function (HKDF)*, H. Krawczyk, P. Eronen, May 2010 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc5869.txt` | 25854 | `7a40eb3835b35fc947eb12a2ed614db079d43b26e50dbc537c31fba16397089c` |
| `rfc5903.txt` | RFC 5903, *Elliptic Curve Groups modulo a Prime (ECP Groups) for IKE and IKEv2*, D. Fu, J. Solinas, June 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5903.txt` | 29175 | `939fab548a6e6bb49a5b3c4dd24a3c5df54a46645447b2d6f4df4fd88ff2d69f` |
| `rfc5952.txt` | RFC 5952, *A Recommendation for IPv6 Address Text Representation*, S. Kawamura, M. Kawashima, August 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5952.txt` | 26570 | `c75e82c5f53bcec8148820fadf0d65935336ee2031fa6ce10504797ed4c1979d` |
| `rfc6668.txt` | RFC 6668, *SHA-2 Data Integrity Verification for the Secure Shell (SSH) Transport Layer Protocol*, D. Bider, M. Baushke, July 2012 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc6668.txt` | 8710 | `a7a05b87ea5dcb16c33f368de8776a194bc35d7fc9d0baea600f3083dbc857b9` |
| `rfc6724.txt` | RFC 6724, *Default Address Selection for Internet Protocol Version 6 (IPv6)*, D. Thaler (ed.), R. Draves, A. Matsumoto, T. Chown, September 2012 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc6724.txt` | 74407 | `deab574626b2bc886748401617f29a33cf97927f3690dba2fb183d31315741f6` |
| `rfc6979.txt` | RFC 6979, *Deterministic Usage of the Digital Signature Algorithm (DSA) and Elliptic Curve Digital Signature Algorithm (ECDSA)*, T. Pornin, August 2013 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc6979.txt` | 140386 | `456e8f17558fdbd206f968b96fc6f1b4a71ea331ab30ad17f711ab3adaa7d701` |
| `rfc7748.txt` | RFC 7748, *Elliptic Curves for Security*, A. Langley, M. Hamburg, S. Turner, January 2016 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc7748.txt` | 39298 | `279ca0ecc5e92e2962e27b846986aeb74729d9dd34bd4a04a362f80dcb596ad3` |
| `rfc8017.txt` | RFC 8017, *PKCS #1: RSA Cryptography Specifications Version 2.2*, K. Moriarty (ed.), B. Kaliski, J. Jonsson, A. Rusch, November 2016 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc8017.txt` | 154696 | `1e72dc473d18df3fc5598cdc12795a9f18f36f1aef15abc23a55eb0d58151d11` |
| `rfc8032.txt` | RFC 8032, *Edwards-Curve Digital Signature Algorithm (EdDSA)*, S. Josefsson, I. Liusvaara, January 2017 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8032.txt` | 103210 | `ed63657ff389301282b169b0abde9b5dd2c7e4d524fdfa5da6ff3094fc93c4c3` |
| `rfc8106.txt` | RFC 8106, *IPv6 Router Advertisement Options for DNS Configuration*, J. Jeong, S. Park, L. Beloeil, S. Madanapalli, March 2017 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8106.txt` | 43092 | `9a44a5e06d36506da358fa0d62d02f484dd76b4e6ca75920ebda2cfed315ef2f` |
| `rfc8200.txt` | RFC 8200, *Internet Protocol, Version 6 (IPv6) Specification*, S. Deering, R. Hinden, July 2017 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8200.txt` | 93162 | `371ae3f133d562db5d6385e6def4ca9914c4f831be228ea7779fd28799c2f490` |
| `rfc8201.txt` | RFC 8201, *Path MTU Discovery for IP version 6*, J. McCann, S. Deering, J. Mogul, R. Hinden (ed.), July 2017 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8201.txt` | 42751 | `96c2ea7ac1bf5810f6b817d4ac372a35f68231d2bd0e2675740f1268eb9ac752` |
| `rfc8268.txt` | RFC 8268, *More Modular Exponentiation (MODP) Diffie-Hellman (DH) Key Exchange (KEX) Groups for Secure Shell (SSH)*, M. Baushke, December 2017 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8268.txt` | 16318 | `1525b5f3e84f48381fc3be2e5353794f3d5ee2d354fa4e8b0349da25d5b8e208` |
| `rfc8308.txt` | RFC 8308, *Extension Negotiation in the Secure Shell (SSH) Protocol*, D. Bider, March 2018 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8308.txt` | 29204 | `c7d36121bcd0f242af19d660a3bbafb3c829d78df6f32fbd3e5c71a408e0ce8c` |
| `rfc8332.txt` | RFC 8332, *Use of RSA Keys with SHA-256 and SHA-512 in the Secure Shell (SSH) Protocol*, D. Bider, March 2018 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8332.txt` | 17873 | `b194d8da7a5b07cf6763954b687fb2380565c8277d52c105a8eacdf00ffc44d9` |
| `rfc8439.txt` | RFC 8439, *ChaCha20 and Poly1305 for IETF Protocols*, Y. Nir, A. Langley, June 2018 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8439.txt` | 88847 | `25bef70fbf7a07ff45c2fe4cb7c6ce954eac687413d8610603268b4e4415324c` |
| `rfc8446.txt` | RFC 8446, *The Transport Layer Security (TLS) Protocol Version 1.3*, E. Rescorla, August 2018 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8446.txt` | 337736 | `47871bc8820a2c3b6ea89f061055577058862cf543686b82d10131239702b3bd` |
| `rfc8448.txt` | RFC 8448, *Example Handshake Traces for TLS 1.3*, M. Thomson, January 2019 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8448.txt` | 159343 | `6564d1376d1ec744fc7a9993da15ebc1b9be361908b166091f47ef605c537fba` |
| `rfc8709.txt` | RFC 8709, *Ed25519 and Ed448 Public Key Algorithms for the Secure Shell (SSH) Protocol*, B. Harris, L. Velvindron, February 2020 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8709.txt` | 11255 | `ea4aa3a8feec6bd5b1cc7847a6d08cd0afae49d12220cac1e2c39bc72c29fd3d` |
| `rfc8731.txt` | RFC 8731, *Secure Shell (SSH) Key Exchange Method Using Curve25519 and Curve448*, A. Adamantiadis, S. Josefsson, M. Baushke, February 2020 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc8731.txt` | 12460 | `26dcf1e8481d83e0b2403e41df47df3fb08cae154d1e464631a23b3cd025f9d1` |
| `rfc9110.txt` | RFC 9110, *HTTP Semantics*, R. Fielding (ed.), M. Nottingham (ed.), J. Reschke (ed.), June 2022 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc9110.txt` | 502941 | `21c1cdce6ab0e5509b04d84a28000836c7a087cf786efe6f04877ebfff47232a` |
| `rfc9112.txt` | RFC 9112, *HTTP/1.1*, R. Fielding (ed.), M. Nottingham (ed.), J. Reschke (ed.), June 2022 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc9112.txt` | 109913 | `e4f426bac6206b67fdf9e0da826154f70588db2133a0a86b15cde4ff725d8937` |
| `rfc9142.txt` | RFC 9142, *Key Exchange (KEX) Method Updates and Recommendations for Secure Shell (SSH)*, M. Baushke, January 2022 | 2026-09-08 from `https://www.rfc-editor.org/rfc/rfc9142.txt` | 52250 | `68226c742986b83511fbc958cfa8bf5b51a8fdefdcae146d7ea41f2e4be79026` |
| `rfc9293.txt` | RFC 9293, *Transmission Control Protocol (TCP)*, W. Eddy, Ed., August 2022 | 2026-09-06 from `https://www.rfc-editor.org/rfc/rfc9293.txt` | 263696 | `6d9ac8be4b0286f8c3d337addf442b2eb6a9b14e1366594ea7fbc273f93dc2d9` |

The checksums are here so that a reader can tell a file has not been
edited. Each is the text as the RFC Editor publishes it, byte for byte,
including the page breaks: 2887, 1218, 470, 171, 3077, 1417, 6844, 619,
955, 619, 2523, 1907, 1067, 395, 1515, 563, 451, 1403, 507, 1123, 1683,
955, 1795, 1347, 1403, 1347, 5435, 1683, 1123, 1123, 339, 451, 787, 899,
787, 283, 1795, 4427, 1235, 4371, 3363, 1067, 2355, 1067, 451, 787, 507,
2579, 8963, 3811, 317, 287, 10785, 2461, 1028, and 5576 lines
respectively, in the order of the table.
Every one was fetched twice and the two fetches agreed.

## Terms

These documents are not covered by this repository's licence. Each
carries a notice of the form

> Copyright (c) YEAR IETF Trust and the persons identified as the
> document authors. All rights reserved.

with the year 2009 for RFC 5480 and RFC 5656, 2010 for RFC 5756,
RFC 5758, RFC 5869, RFC 5903, and RFC 5952, 2012 for RFC 6668 and
RFC 6724, 2013 for RFC 6979, 2016 for RFC 7748 and RFC 8017, 2017 for
RFC 8032, RFC 8106, RFC 8200, RFC 8201, and RFC 8268, 2018 for RFC 8308,
RFC 8332, RFC 8439, and RFC 8446, 2019 for RFC 8448, 2020 for RFC 8709
and RFC 8731, and 2022 for RFC 9110, RFC 9112, RFC 9142, and RFC 9293.
Those twenty-eight are subject to BCP 78 and the IETF Trust's Legal
Provisions relating to IETF Documents, which permit reproduction in
full.

RFC 4055 and RFC 4231 stand between the two forms. Each names the
Internet Society and the year 2005, and then refers the reader to BCP 78
in the same words the later ones use, so both are read here as the group
above rather than the group below.

The twenty-four older ones carry the notice of their time, each in a
full copyright statement at the end that permits reproduction in whole
provided the notice travels with it. RFC 4862 and RFC 4861 have the IETF
Trust's of 2007; RFC 4443, RFC 4291, and the five Secure Shell documents
the Internet Society's of 2006; RFC 3596 and RFC 3526 the Internet
Society's of 2003; RFC 3279 the Internet Society's of 2002; RFC 2464 and
RFC 2313 the Internet Society's of 1998; and RFC 1122 carries that
statement in the form of 1989. RFC 1071, RFC 1035, RFC 2104, RFC 2131,
RFC 2132, RFC 894, RFC 826, RFC 792, and RFC 791 carry no notice at all.
RFC 1950 and RFC 1951 carry one of a third kind: a notice of their
authors' own, dated 1996, which grants the right to copy and distribute
the document in any medium provided it is not modified and the notice
travels with it; both also state unlimited distribution in their Status
of This Memo. Each of the first five states unlimited distribution in
its own Status of This Memo section, and the four from the early
eighties predate even that form, under the practice the RFC Editor
states for the series as a whole.

Code components extracted from an RFC carry the Simplified BSD Licence;
this project extracts test vectors, which it transcribes into Rust source
with the document and section named at each table, as decision D-40
requires.

## The two documents of DEFLATE

These arrived with the compression of the written documents, and they are
what `audhsos-deflate` implements.

**RFC 1951** is the format itself: a stream of blocks, each one stored,
coded with a table the format fixes, or coded with a table the block
carries; and, inside a coded block, literal bytes and back-references into
what has already been decoded. What makes the document worth having in
the house rather than summarised is section 3.2: the rule that turns a set
of code lengths into the one canonical code they stand for, and the three
tables — the lengths, the distances, and the order the code lengths of a
dynamic block are written in — where every number is load-bearing and none
of them can be derived from anything else.

**RFC 1950** is the wrapper: two header bytes that say which method and
what window, and an Adler-32 of the uncompressed data at the end. It is
six pages, and the reason it is here is the checksum, which is stated as
the algorithm it is rather than by reference to anywhere else.

The pair is what a PDF calls `FlateDecode` and what a PNG carries in its
image chunks, which is why the third document below sits beside them.

## Why RFC 1071

The internet checksum is one algorithm used by IP, ICMP, UDP, and TCP, and
`net-wire` computes it for all four. Section 3 of the memo is a worked
example with every intermediate value written out — the same eight bytes
summed byte by byte, as 16-bit words in both byte orders, and as 32-bit
words in three orders, followed by the same sum split into two groups
across an odd boundary. That last table is the one that checks an
accumulator which carries a pending byte from one chunk to the next, and
it is not in RFC 791, RFC 768, or RFC 9293, which state the checksum and
give no numbers for it. The vectors of catalog 6.6.42 are transcribed from
it.

The memo is not a standard; it says so itself. What is normative is that
the sum is the 16-bit one's complement of the one's complement sum, and
that comes from the protocol specifications. This document is kept for its
numbers and for section 2, which states why the sum may be computed in
either byte order and in any grouping — the property the incremental
accumulator rests on.

## The three documents of IPv4

**RFC 791** is the protocol: the twenty-byte header and its fields, the
rule that options are a whole number of words and may be skipped by a
host that does not implement them, and — section 3.2, *Fragmentation and
Reassembly* — the identification, flags, and offset that a datagram is
cut and put together by, with the reassembly procedure written out as an
algorithm.

**RFC 792** is ICMP: the echo request and reply, destination unreachable
with its codes, and time exceeded, each carrying the header of the
datagram that caused it plus eight bytes, which is what lets the upper
layer match an error to the connection that earned it.

**RFC 1122, section 3.2.2** is the list this implementation follows most
carefully: an ICMP error is never sent in answer to an ICMP error, to a
datagram addressed to a broadcast or multicast address, to one that
arrived as a link-layer broadcast, to a non-initial fragment, or to one
whose source address names no single host. The memo says these
restrictions take precedence over every other requirement to send an
error, and it explains why — a broadcast to a closed port would otherwise
draw an answer from every host on the link at once.

## The four documents of the link layer

These are what `net-eth` implements.

**RFC 894** is three pages and settles the frame: an IP datagram on an
Ethernet is carried in an Ethernet II frame with type `0x0800`, ARP with
`0x0806`, and the payload is at most 1500 bytes. It is where the MTU
comes from, and it says the trailing checksum belongs to the hardware,
which is why nothing in this crate computes one.

**RFC 826** is the address resolution protocol: the 28-byte packet for
Ethernet and IPv4 with its six fields, the rule that a reply is sent only
by the station that owns the target address, and the observation — the
one this implementation follows most closely — that a station may learn a
mapping from any packet it sees rather than only from a reply it asked
for. What the memo does not say is what to do when a second station
claims an address a first one already answered for, which is where the
project's own rule comes in: a reachable entry is never overwritten with
a different hardware address.

**RFC 4861, section 7.3** is the neighbor cache: the five states
`INCOMPLETE`, `REACHABLE`, `STALE`, `DELAY`, and `PROBE`, what moves an
entry between them, and the constants of section 10 — `REACHABLE_TIME`
30 seconds, `RETRANS_TIMER` 1 second, `DELAY_FIRST_PROBE_TIME` 5 seconds,
and three solicitations of either kind. The cache here is one cache for
both families (D-69), so ARP fills it with the three states it needs and
Neighbor Discovery, in `net-ipv6`, uses all five.

**RFC 2464** carries IPv6 over an Ethernet, and it is three paragraphs of
this crate. Section 7 maps a multicast group onto a hardware address by
arithmetic — `33:33` and the last four bytes of the group — which is what
lets a solicitation reach a station nobody has an address for, where ARP
has to broadcast to the whole link. Section 4 forms the interface
identifier SLAAC puts behind a prefix: the OUI, `FF FE`, the rest of the
address, with the universal/local bit complemented, and it works the
example this project transcribes. And it fixes the prefix length that
autoconfiguration works at, at 64 bits, which is the number `net-ipv6`
refuses to form an address under any other.

## The three documents of IPv6 addressing

These arrived with D-69, which put IPv6 into the first network version
beside IPv4. They are what `net-wire` implements, and RFC 8200 came with
them because its section 8.1 is what a checksum over IPv6 is summed with.

**RFC 4291** is the address architecture: the 128-bit address, the text
forms of section 2.2, the prefixes that make an address unspecified,
loopback, link-local, unique-local, or multicast, and — section 2.7.1 —
the solicited-node multicast address, which is the low 24 bits of a
unicast address appended to `ff02::1:ff00:0/104`. Neighbor Discovery is
addressed to that group rather than to a broadcast, which is the reason
`Ipv6Addr::solicited_node` exists before there is a neighbor to discover.

**RFC 5952** is the canonical text form, and the reason the address type
has one spelling per value. Section 4 is four rules: leading zeros
suppressed, `::` used to its maximum, `::` never used for a single zero
field, and on a tie the leftmost run shortened; section 4.3 requires
lower case. RFC 4291 permits several texts per address; this document
picks one of them, and the parser here refuses the others (D-69).

**RFC 8200, section 8.1** is the pseudo-header UDP, TCP, and ICMPv6
compute their checksums over: the two 128-bit addresses, a 32-bit
upper-layer length, three zero bytes, and the next-header value of the
upper-layer protocol — which is not the next-header field of the packet
when extension headers stand between them. The rest of the document is
the header format and the extension header chain, which the IPv6 step
will read.

## The four documents of IPv6 itself

These are what `net-ipv6` implements, beside RFC 8200 above. Every one of
them is a protocol IPv4 either has no counterpart for or solves somewhere
else, which is why the family needed four documents where the addressing
took three.

**RFC 4443** is `ICMPv6`: echo request and reply, destination unreachable
with its seven codes, time exceeded, and — the one with no counterpart at
all — packet too big, which carries the MTU of the link a packet did not
fit. Two rules of it are not cosmetic. Section 2.3 sums the checksum over
the pseudo-header of both addresses, where `ICMPv4`'s covers the message
alone; a corrupted address is caught there because the IPv6 header has no
checksum to catch it. And section 2.4 (e) forbids answering an error, a
multicast destination, a link-layer multicast, or a source that names no
single node with an error of one's own, in the way RFC 1122, section 3.2.2
does for IPv4, and for the same reason.

**RFC 4862** is stateless address autoconfiguration: how a prefix out of
a router advertisement and an interface identifier become an address, the
valid and preferred lifetimes that address holds for, and — sections 5.4
to 5.4.5 — duplicate address detection, which is the check that has to
pass before the address is used at all. It is also where the rule lives
that an address is formed only when the prefix length and the identifier
make 128 bits together (section 5.5.3 (d)), which for an Ethernet is a
prefix of 64 and nothing else.

**RFC 8106** is one option, and it is what makes DHCPv6 unnecessary here
(D-69). Section 5.1 puts recursive DNS servers into a router
advertisement: a type, a length in eight-byte units, a lifetime, and one
or more addresses, with the count read back out of the length. Without
it a host that configured itself from an advertisement would still have
to speak a second protocol to learn where to resolve names.

**RFC 8201** is path MTU discovery, and section 4 is the whole of what
this system implements: the estimate is lowered by a packet-too-big
message, never raised by one, never taken below the minimum link MTU of
1280, and tried again no sooner than five minutes later — ten being the
recommended setting, and the one used. It is not optional for IPv6 the
way it is for IPv4: RFC 8200, section 4.5 forbids a router to fragment,
so a packet that does not fit is not cut up on the way, it is returned as
this message or it is lost.

## Why RFC 8448 in particular

It is the only published trace of a complete TLS 1.3 handshake: every
secret, every message, and every record of one connection, with the
private keys that produced them. A client that reproduces it byte for
byte has been checked against an implementation that was not this one,
which no amount of testing a client against its own idea of a server can
do.

## Why RFC 9293

It is the whole of TCP in one document. RFC 793 had been amended by seven
memos over forty years — the initial sequence number of RFC 6528, the
urgent pointer of RFC 6093, the reset checks of RFC 5961 among them — and
RFC 9293 is those amendments folded back into one text, which is why it
obsoletes all seven. An implementation written against the old memo and
its errata is an implementation whose reader has to know which of them
applied; written against this one, the state machine of section 3.3.2,
the segment-arrives procedure of section 3.10.7, and the acceptance test
of section 3.4 are read where they stand.

What it does not carry is the two mechanisms that surround the state
machine and are specified elsewhere: the retransmission timer of RFC 6298
and the congestion control of RFC 5681. This implementation follows both,
and cites them by section where it does.

## The two documents of DNS

**RFC 1035** is the format and the resolver: the header of section 4.1.1,
the question of 4.1.2, the resource record of 4.1.3, and the compression
of 4.1.4, with the size limits of section 2.3.4 and the case rule of
2.3.3. It is kept for four passages that a summary cannot stand in for.
Section 4.1.4 states that a pointer replaces "a list of labels at the end
of a domain name" — the end, which is why a pointer is the last thing in a
name and why the name goes on where the pointer stood and not where it
pointed. Section 2.3.4 gives the two numbers a reader has to enforce while
it assembles a name and not afterwards: 63 for a label, 255 for a name.
Section 3.3.1 gives `CNAME` a body that is a name, which is what makes the
alias chain a chain and not a string. And section 4.2.1 puts a UDP answer
at 512 bytes, which is the whole of why the truncation bit exists and the
whole of what this resolver cannot do about it (there is no TCP here).

What the document does not carry is the rule this crate follows for
believing a response. RFC 1035 was written when a resolver's only worry
was a lost datagram; the four things that have to agree — the id, the
question, the source address and the source port — are RFC 5452, and only
the last of them is a real addition, the other three being the document's
own matching rules read as a security property. That memo is cited where
it is followed and is not kept here, because it states a requirement and
gives nothing to check against.

**RFC 3596** is three pages of the four that matter: the `AAAA` type is
28, its body is sixteen bytes in network order, and a query for it is a
query like any other. It obsoletes RFC 1886, which had the same type
number and a different `IP6.INT` reverse zone that nothing in this system
asks about. It is kept rather than summarised because the alternative is a
constant with no source, and because section 2.2 is what says the body is
the address itself and not a text form of it.

## The two documents of DHCP

**RFC 2131** is the protocol: the message of section 2, the exchange of
section 3.1, the state machine of figure 5, and the client behaviour of
section 4. Three passages of it are the reason the file is here rather
than a summary. Section 4.1 states the deadlock the BROADCAST flag exists
for, in the words of a document that had watched it happen — a client that
cannot accept a unicast datagram before it is configured cannot be told
what to configure — and it is the source for the decision to set the flag
until the address stands. The same section gives the backoff in numbers:
four seconds, doubled to sixty-four, each delay moved by a uniform value
between minus one and plus one second. And section 4.4.5 gives the two
lease timers their defaults — half the lease and seven eighths of it — and
the retransmission rule that replaces the backoff once a lease exists:
half of what is left until the next deadline, never below sixty seconds.

**RFC 2132** is the option catalog. What is taken from it is small — the
magic cookie of section 2, pad and end of 3.1 and 3.2, the subnet mask of
3.3, the routers of 3.5, the name servers of 3.8, and the seven DHCP
options of section 9 — but the numbers are only in this document, and the
shape of the walk is only in section 2: code, length, body, with two codes
that break that shape and are named there and nowhere else. Section 3.5
also states that the routers come in order of preference, which is why the
first of them is the one used and the rest are read past.

What is deliberately not read is the option overload of section 9.3, which
lets a server put options in the `sname` and `file` fields when the option
field runs out. The six options a lease needs come to under forty bytes,
and the option field holds three hundred.

## Why RFC 6724

It is the only document that says which of a host's addresses a packet
leaves with, and it is the reason a dual-stack host reaches a name over
the family it should. What is kept here rather than summarised is the
policy table of section 2.1 and the two rule lists of sections 5 and 6:
the table is nine rows of numbers that exist nowhere else, and the rules
are an ordered list where the order is the whole of the meaning.

One thing about it had to be read rather than assumed. The table is
written over IPv6 prefixes with IPv4 present as `::ffff:0:0/96`, the
mapped range — and D-69 refuses mapped addresses outright, so the table as
written cannot be applied to an IPv4 address of this system at all. What
the document actually needs from that row is a precedence and a label for
the IPv4 family, which is what this implementation reads it as (D-88).
Section 3.1 settles the other thing a summary gets wrong: an IPv4 private
address has *global* scope, and only the loopback and link-local ranges do
not.

Happy Eyeballs (RFC 8305) is not here and is not needed: this stack tries
a name's addresses in the order this document gives and does not race
them, which section 1 of RFC 8305 is itself a departure from.

## The two documents of HTTP

**RFC 9112** is the wire format: the request line, the field lines, the
chunked coding of section 7.1, and — the reason the file is here — the
message body length of section 6.3. That section is an ordered list of
eight rules, and an implementation that reads them out of order reads a
different protocol. Point 3 is the one this client departs from and says
so: it lets a recipient prefer `Transfer-Encoding` over `Content-Length`
and states in the same paragraph that such a message ought to be handled
as an error. This client does the latter, because a preference rule is a
second reading with a tie-breaker rather than one reading, and two
readings of one message is the whole of request smuggling. Point 8 is what
makes `Decoder::finish` necessary: a response with no declared length runs
until the connection closes, so a caller has to say when it did.

Section 5.2 is the other passage read at the source. It has a user agent
*replace* an obsolete line fold with spaces rather than refuse the
message; this client refuses it, and the deviation is recorded rather than
discovered.

**RFC 9110** is the grammar and the semantics. What is taken from it is
small and exact: the `tchar` set of section 5.6.2, which is what a field
name may be made of; the field value of section 5.5, which is visible
ASCII, the space and the tab, with `obs-text` allowed and deprecated in
one sentence — this client refuses `obs-text`, which is what makes every
value it hands out text and not bytes; the case-insensitive comparison of
field names in section 5.1; and the status classes of section 15, of which
the five redirects a client may follow are 301, 302, 303, 307 and 308 and
not the whole 3xx range.

## The four documents of P-384

These four are what P-384 took, and no more. They were gathered because
`crypto-ec` had P-256 and Ed25519 and no P-384, which is where a real
chain stopped: Google Trust Services issues from a P-256 intermediate
under a P-384 root, so `SubjectPublicKey::parse` reached `GTS Root R4`,
saw a curve that was not `prime256v1`, and answered
`UnsupportedAlgorithm`. Every anchor above such an intermediate was out of
reach. The curve is implemented now, and these are the documents its
constants and its vectors come from.

**RFC 5903, section 3.2** — the numbers. The prime
`p = 2^384 - 2^128 - 2^96 + 2^32 - 1`, the curve `y^2 = x^3 - 3x + b`
with `b` in full, the generator `g = (gx, gy)`, the group order, and the
seed the curve was verifiably generated from. Section 5, *Alignment with
Other Standards*, is the table that says the 384-bit random ECP group,
NIST P-384, and SECG `secp384r1` are one curve under three names. RFC 6090 points here for the parameter set,
which is why this is the copy that is kept.

**RFC 5480** — how the key is written down. Section 2.1.1.1 gives
`secp384r1` the identifier `1.3.132.0.34`; section 2.2 gives the
uncompressed point encoding, `0x04` followed by `x` and `y`, which for
P-384 is 97 bytes where P-256 takes 65; section 4 pairs a key of 384 bits
with SHA-384. The ASN.1 module also carries
`ECDSA-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }`, the shape a
signature arrives in. This is the document `SubjectPublicKey::parse`
implements, and the one place it has to change.

**RFC 5758, section 3.2** — `ecdsa-with-SHA384` is
`1.2.840.10045.4.3.3`, and its `AlgorithmIdentifier` omits the parameters
field rather than encoding a null. This half was already done when the
others were not: `oid::ECDSA_WITH_SHA384` and
`SignatureAlgorithm::EcdsaSha384` existed and were exercised. The document
is kept because the identifier and the key it names belong to one change,
and a reader of the other three should not have to go looking for it.

**RFC 6979, appendix A.2.6** — the vectors. A P-384 key pair given as the
private scalar `x` and the public point `Ux, Uy`, the group order `q`,
and ten signatures: `k`, `r`, and `s` for SHA-1, SHA-224, SHA-256,
SHA-384, and SHA-512, over each of the messages `"sample"` and `"test"`.
It is the same table one curve up from the one the P-256 code is already
checked against: `crates/crypto/ec/src/tests/p256.rs` transcribes
appendix A.2.5, and `the_rfc_6979_vectors_sign_and_verify_as_documented`
is the test that reads it.

Note what the appendix does not give: it states the order `q` but not
`p`, `b`, or the generator, so a verifier tested against it is tested
against RFC 5903 as well. The two are one vector set.

## What P-384 does not need, and why it is not here

These were read and left out. They are named so that the next reader does
not repeat the search.

- **RFC 5114, section 2.7** states the same P-384 constants as RFC 5903,
  with `a` written out in hexadecimal instead of as `-3`. Comparing the
  two catches a transcription slip, but it is not a second source: both
  are transcriptions of FIPS 186 and SEC 2. Read it there if the check is
  wanted; it earns no copy.
- **RFC 3279** defines `ECDSA-Sig-Value`, but RFC 5480 updates it and
  carries the same definition in its own module. P-384 took nothing from
  it. It is in the table now for a different reason, which the section
  below gives: section 2.3.1 is where `rsaEncryption` and `RSAPublicKey`
  are written down, and no later document restates them.
- **RFC 6090** gives elliptic curve algorithms without naming P-384, and
  points at RFC 5903 for the parameters.
- **RFC 8422** is ECC for TLS 1.2 and earlier. This client speaks 1.3
  only.
- **RFC 9500** carries an encoded P-384 test key. This repository builds
  the certificates it tests with, in `audhsos-x509`'s builder.
- **RFC 8446** assigns `secp384r1` the group `0x0018` and
  `ecdsa_secp384r1_sha384` the scheme `0x0503`. P-384 here is for reading
  a chain, not for the key exchange, which decision D-56 settles on
  `x25519` alone. The TLS 1.3 specification itself was a separate
  question from this one; it has since been answered, and RFC 8446 is in
  the table.

## The five documents of RSA

These five are what RSA verification takes. They were gathered for the
reason the P-384 set was gathered: a real chain stops without them.
Document 11 names RSA as the one omission that costs interoperability, and
it costs it in the same place as before — `SubjectPublicKey::parse` meets
`rsaEncryption`, answers `UnsupportedAlgorithm`, and every chain above
that certificate is out of reach. `www.ietf.org`, `www.rust-lang.org`, and
`www.bbc.co.uk` are three such chains.

**RFC 8017** — the arithmetic and the two encodings. Section 5.2.2 is
RSAVP1, the verification primitive, and sections 4.1 and 4.2 are the
conversions between integers and octet strings that bracket it. Section
8.2.2 is RSASSA-PKCS1-v1_5 verification and section 9.2 the encoding it
compares against; note 1 after section 9.2 writes out the DER encoding of
`DigestInfo` for each of nine hash functions, byte for byte, and three of
those lines become constants in the source. Section 8.1.2 is RSASSA-PSS
verification, section 9.1.2 EMSA-PSS-VERIFY, and appendix B.2.1 MGF1.

Section 8.2.2 also settles the shape the implementation takes. Its steps
three and four build the expected encoded message and compare it with the
one recovered from the signature. The note after step four offers the
other form — decode the recovered message and compare the hash inside it —
and weighs the two by storage against code size, saying nothing about what
a decoder gets wrong. That is this project's reason rather than the
document's: a decoder is where a signature with slack in its padding is
accepted, and building the encoding leaves no slack to accept.

**RFC 4055** — what an X.509 certificate calls these. Section 5 assigns
`sha256WithRSAEncryption`, `sha384WithRSAEncryption`, and
`sha512WithRSAEncryption` the arcs `{pkcs-1 11}`, `{pkcs-1 12}`, and
`{pkcs-1 13}`, and settles a question the current parser answers wrongly
for these algorithms: the parameters MUST be NULL, and an implementation
MUST accept them absent as well as present. `SignatureAlgorithm::parse`
calls `finish` on the identifier's fields today and so accepts only
absence, which is right for ECDSA and Ed25519 and wrong here. Section 3.1
gives `id-RSASSA-PSS` and the `RSASSA-PSS-params` syntax, section 2.2 the
`id-mgf1` mask generation function, and section 6, the ASN.1 module, the
parameter sets that pair a hash with MGF1 over that same hash and a salt
as long as its output.

**RFC 5756** — the correction to the one above. It updates RFC 4055 on
where `RSASSA-PSS-params` must appear and where it merely may, in the
signature field, the signature algorithm field, and the subject public key
information. RFC 4055 alone reads as demanding the structure in places
where it is optional, so the first document without the second states a
rule the world does not follow.

**RFC 3279, section 2.3.1** — the key itself. `rsaEncryption` is
`{pkcs-1 1}`, its parameters field MUST have ASN.1 type NULL, and the
subject public key is the DER encoding of
`RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }`.
RFC 4055 adds identifiers to this profile but does not restate the key, so
this is the only document here that says what an RSA
`SubjectPublicKey` parses.

**RFC 2313** — the shape this project does not implement. It is not
needed to write the code, and that is worth saying plainly: nothing in
RFC 8017 is delegated to it. Its normative references are three, and none
of them is a PKCS #1 document; RFC 2313, RFC 2437, and RFC 3447 are all
informative. Section 9.2 of RFC 8017 writes the encoding operation out in
six steps and appendix A.2.4 carries the ASN.1.

What this document holds is the source for a decision. RFC 8017's note 2
after section 9.2 says version 1.5 defined `T` as the BER encoding rather
than the DER encoding, and offers a BER-decoding verifier to anyone who
wants compatibility with it. D-80 declines that offer, and declining it
means refusing signatures another specification calls valid — which is a
claim about a document, and a claim about a document is checked against
the document.

It reads stronger from the source than from the summary. Section 10.1.2
has the `DigestInfo` "BER-encoded to give an octet string D"; section
10.2.3 has that data "BER-decoded to give an ASN.1 value of type
DigestInfo, which shall be separated into a message digest MD and a
message-digest algorithm identifier"; section 10.2.4 then compares the
digests. So version 1.5 does not merely permit a laxer encoding — its
verification *is* the decoder, in its own sections, and the two documents
describe two different operations rather than one operation with a
tolerance. Refusing a `DigestInfo` that is BER but not DER is therefore a
deliberate incompatibility with PKCS #1 v1.5, and 11.15 says so.

Two things the document does not have, which is why the choice costs
nothing else. It carries no table of `DigestInfo` prefixes — no byte
string appears anywhere in it — so the constants of RFC 8017 section 9.2
note 1 have exactly one source. And section 10.2.3 makes it an error if
the digest algorithm is not MD2, MD4, or MD5: version 1.5 predates SHA,
so nothing this system verifies could be read out of it in the first
place.

## What RSA does not need, and why it is not here

These were read and left out, in the form of the section above.

- **RFC 3447**, PKCS #1 version 2.1, is obsoleted by RFC 8017, which says
  so in its header.
- **RFC 2437**, PKCS #1 version 2.0, is on no path at all. RFC 4055 never
  names it, and the version brought OAEP encryption; PSS arrived one
  version later, in RFC 3447, which RFC 8017 obsoletes.
- **RFC 8446, section 4.2.3** assigns the six code points —
  `rsa_pkcs1_sha256`, `sha384`, `sha512`, and `rsa_pss_rsae_sha256`,
  `sha384`, `sha512` — and states that the `rsa_pkcs1_*` schemes stand for
  certificate signatures and MUST NOT appear in a `CertificateVerify`.
  That asymmetry is the whole of what the protocol adds here. Whether the
  TLS 1.3 specification belonged in this directory was the open question
  the P-384 section left, and adding RSA did not answer it; it is
  answered now, and the document is here.
- **NIST CAVP** signature verification vectors and **FIPS 186-4** are not
  RFCs. This project cites NIST publications by section where it uses
  them — FIPS 197 for AES, SP 800-38D for GCM — and keeps no copies. The
  practice does not change for RSA.
- **A test key** is not a document, and there is no need to fetch one.
  RFC 8448 section 2 already prints a complete RSA private key — modulus,
  public exponent, private exponent, both primes, and the Chinese
  remainder values — and the traces it belongs to carry real
  `rsa_pss_rsae_sha256` signatures made with it. The modulus is 1024 bits,
  which is below what this system will accept from a certificate, so the
  key is a vector for the primitive and not a chain anything would trust.
  That is what it is wanted for.

## The five documents of Secure Shell

These five were published together in January 2006 and are one
specification cut into five files: each of the other four is written in
the types RFC 4251 defines and uses the constants RFC 4250 assigns.
Nothing in this repository implements them yet; they are here so that the
work can be written against the text rather than against a recollection
of it.

**RFC 4251** is the architecture, and section 5 is the reason it is read
first: the wire types the whole protocol family is spelled in — `byte`,
`boolean`, `uint32`, `uint64`, `string`, `mpint`, and `name-list`. A
`string` is a `uint32` length and that many bytes, which may be arbitrary
binary and are not null-terminated. An `mpint` is that same string
holding a two's complement integer, most significant byte first, with a
zero byte prefixed when a positive number's top bit would otherwise be
set, no unnecessary leading `00` or `ff`, and zero encoded as a string of
no bytes at all. The section works five of those encodings out in hex,
and a `name-list` in three more, which is what makes the file worth
keeping: the rule can be paraphrased, the table checks an encoder.
Section 6 fixes how algorithm names are spelled — a name with no `@` is
one IANA assigns, a name with one is the local namespace of whoever owns
the domain after it.

**RFC 4253** is the transport layer, and its load-bearing passages have
to be read rather than recalled. Section 4.2 is the identification string,
`SSH-2.0-softwareversion` with an optional comment, terminated by CR LF,
at most 255 characters including those two — and the part *before* the CR
LF is what goes into the exchange hash, which is why it has to be kept
after it has been parsed. Section 6 is the binary packet: a `uint32`
packet length, a padding length byte, the payload, at least four bytes of
random padding, and the MAC, with the length of everything but the MAC a
multiple of the cipher block size or eight, whichever is larger — a
constraint the document requires even of a stream cipher — and with the
packet length field itself encrypted, which is the whole reason a reader
has to decrypt one block before it knows how much more to read. Section
6.4 computes the MAC over `sequence_number || unencrypted_packet`, where
the sequence number is a `uint32` that never appears on the wire, starts
at zero, is never reset by a re-exchange, and wraps at 2^32. And section
7.2 derives six keys — two IVs, two encryption keys, two integrity keys —
as `HASH(K || H || X || session_id)` for the single letters `A` through
`F`, with the extension rule that appends `HASH(K || H || <key so far>)`
until enough bytes exist. Section 8 gives the exchange hash those rest
on: `H` over `V_C || V_S || I_C || I_S || K_S || e || f || K`, and states
that `H` from the *first* exchange becomes the session identifier and
does not change afterwards, however often the keys do.

Two smaller rules of it are the kind that are wrong when guessed. Section
6.1 sets the sizes every implementation must be able to receive: an
uncompressed payload of 32768 bytes and a total packet of 35000. And
section 7.1 defines the negotiation, including the guessed packet: a peer
may send its first key exchange packet before it knows the answer, and if
the guess was wrong that packet is *silently ignored* rather than
treated as an error.

**RFC 4252** is authentication. The framework of section 4 is a request
naming a user, a service, and a method, answered by a failure that lists
what may still be tried and carries a partial-success flag, or by a
success that ends authentication for the connection. What matters most is
the signature of section 7: it is computed over the session identifier
followed by the request fields, so a signature captured from one
connection proves nothing on another. Section 5 also settles what a
server does with a user name it does not know: it may disconnect, or it
may send back a bogus list of methods that can continue, so that the
answer does not say which accounts exist — but it must never accept the
request.

**RFC 4254** is the connection protocol, and it is where a single
encrypted stream becomes many. A channel is opened by either side with
its own number, an initial window, and a maximum packet size; section 5.2
makes the window a credit the sender spends and the receiver grants back
with `SSH_MSG_CHANNEL_WINDOW_ADJUST`, up to 2^32 - 1 and never beyond it.
Extended data — stderr is type 1 — spends the same window as ordinary
data, which is the detail a second buffer would get wrong. On top of that
sit the session channel of section 6 with its pty request, environment,
shell, command, and subsystem requests and its exit status, and the TCP
port forwarding of section 7. Section 8 is the encoded terminal modes: an
opcode byte, and for opcodes 1 to 159 a `uint32` argument, ended by
opcode zero.

**RFC 4250** is the numbers: the disconnect reason codes, the channel
open failure codes, the extended data type codes, the terminal mode
opcodes of section 4.5, and the name registries for services,
authentication methods, channel types, global and channel requests,
signals, subsystems, key exchange methods, and the four algorithm
classes. Some of these tables also appear in the document that uses
them — the terminal modes are in RFC 4254, section 8 as well, and RFC
4253, section 12 summarises the message numbers — but section 4.1.1 is
only here, and it is what makes the numbers readable: 1 to 19 transport
generic, 20 to 29 algorithm negotiation, 30 to 49 specific to a key
exchange method, 50 to 59 authentication generic, 60 to 79 specific to
an authentication method, 80 to 89 connection generic, 90 to 127
channels, 128 to 191 reserved, 192 to 255 local. The two method-specific
ranges are reused by every method, so a byte in them means nothing until
one knows which method is running.

## The seven documents of the algorithms Secure Shell is run with

The five above are the framework and a set of algorithms that has been
replaced almost entirely. RFC 4253 makes `ssh-dss` REQUIRED, `3des-cbc`
REQUIRED and `hmac-sha1` REQUIRED, and offers `diffie-hellman-group1-sha1`
and `diffie-hellman-group14-sha1` as its two key exchanges. None of that
is implemented here, and `crates/crypto` holds none of the primitives it
would take: no SHA-1, no 3DES, no CBC mode, no DSA. These seven are what
stands in its place.

**RFC 9142** is why that is a smaller departure than it reads. It is
Standards Track, it updates RFC 4250 and RFC 4253, and its table 12
restates the requirement level of every key exchange method named in any
of them: `diffie-hellman-group1-sha1` goes from MUST to SHOULD NOT,
`diffie-hellman-group14-sha1` from MUST to MAY, `ecdh-sha2-nistp256` and
its siblings from MUST to SHOULD, and `curve25519-sha256` — which had no
recommendation at all — to SHOULD. The two SHA-1 exchanges this system
refuses are the two the IETF itself has withdrawn. What the same table
puts in their place is `diffie-hellman-group14-sha256`, which it makes
the one MUST, and that one this system does not implement either; the
document is kept so that the departure can be stated as what it is
rather than as a list of things that were skipped. Section 3.5 also
makes `ext-info-c` and `ext-info-s` SHOULD, which is why RFC 8308 is
here. Keeping RFC 4253 without this document would be keeping a text
whose requirement levels no longer hold, which is the reason RFC 9293 is
here instead of RFC 793.

**RFC 8731** is `curve25519-sha256`, and section 3.1 is the passage that
makes it worth a file. X25519 produces 32 bytes, which RFC 7748 defines
as a little-endian encoding; SSH reinterprets those same bytes as a
big-endian unsigned integer and then encodes that integer as an `mpint`
under section 5 of RFC 4251. So the shared secret enters the exchange
hash with a leading zero byte, or without one, according to the top bit
of a value that is uniform. An implementation that feeds the 32 bytes in
as a fixed-length string instead computes a different hash from its peer
on about half of its connections, and the handshake fails on those and
succeeds on the rest. Section 3 adds the two aborts: a shared
secret of all zeros, and a received public key that is not 32 bytes, each
a disconnect with `SSH_DISCONNECT_KEY_EXCHANGE_FAILED`.

**RFC 8709** is `ssh-ed25519`, and of its eleven kilobytes four lines are
what this system needs: the public key is `string "ssh-ed25519"` followed
by a `string` holding 32 octets, and the signature is `string
"ssh-ed25519"` followed by a `string` holding 64. Everything else —
generation, signing, verification — it defers to RFC 8032. Those two
encodings are stated nowhere else, which is the whole reason for the
copy.

**RFC 8332** is `rsa-sha2-256` and `rsa-sha2-512`, and it is kept for the
asymmetry that catches every first implementation: the *key* blob keeps
the string `"ssh-rsa"`, so a stored key and its fingerprint do not
change, while the *signature* blob names `"rsa-sha2-256"`. The algorithm
name in a `SSH_MSG_USERAUTH_REQUEST` and the name inside the key it
carries are therefore deliberately different. Signing is RSASSA-PKCS1-v1_5
of RFC 8017 with SHA-256 or SHA-512, and section 3.3 states why this
document does not stand alone: servers penalise clients that offer a
signature algorithm they do not accept, so a client that cannot ask first
should not guess.

**RFC 8308** is how it asks. A client puts `ext-info-c` into the
`kex_algorithms` name-list and a server puts `ext-info-s` — into that
list because it is one of the two in `SSH_MSG_KEXINIT` with no separate
copy per direction, and with different spellings for the two roles
precisely so that they can never match each other and so can never be
chosen as the key exchange. What comes back is `SSH_MSG_EXT_INFO` with
`server-sig-algs`, the list of signature algorithms the server will
accept for authentication.

**RFC 6668** is `hmac-sha2-256` and `hmac-sha2-512` with their digest and
key lengths, and it is here for a path this system does not currently
take. With an AEAD cipher there is no separate MAC to negotiate; the
document is the standing alternative if that ever changes, and it is five
pages.

**RFC 5656** is the NIST-curve family, `ecdh-sha2-*` and `ecdsa-sha2-*`:
names formed by appending a curve identifier, a key blob that nests the
identifier and the point `Q` inside a blob of its own, a signature blob
holding `mpint r` and `mpint s`, and a hash chosen by the size of the
curve. It is the one of the seven that does not stand alone. Point
encoding, public key validation, cofactor ECDH and the conversion from a
field element to an integer are all in SEC 1, which is a SECG document
and is not in this directory.

## What Secure Shell still needs, and why it is not here

- **`chacha20-poly1305@openssh.com`** is the cipher this work will use,
  and it has no RFC. It is specified in `PROTOCOL.chacha20poly1305` in
  the OpenSSH source, which is a document but not a standards body's.
  Whether it is kept the way D-100 added `docs/oasis/` for a second
  body, or cited the way D-124 cites what PCI-SIG releases only to
  members, is an open decision. The same applies to
  `aes128-gcm@openssh.com` and `zlib@openssh.com`.
- **RFC 5647** is AES-GCM for Secure Shell, and it was read and left out.
  Its section 7.3 is worth knowing — the packet length field becomes
  additional authenticated data rather than plaintext, because a tag
  cannot be verified before the packet is parsed and the packet cannot be
  parsed before the length is decrypted — but its algorithm names are
  `AEAD_AES_128_GCM` and `AEAD_AES_256_GCM`, which the OpenSSH this
  system is tested against does not offer under those names.
- **RFC 4344** is the CTR modes, `aes128-ctr` and its siblings. It is the
  only cipher document whose names both an RFC and OpenSSH agree on, and
  it is left out because CTR carries a separate MAC, which means the
  construction of RFC 4253, section 6: MAC-then-encrypt over an encrypted
  length. The encrypt-then-MAC variants that repair it are OpenSSH names
  without a document either.
- **`diffie-hellman-group14-sha256`** is the one MUST of RFC 9142's
  table, and the two documents it needs — RFC 8268 for the name and
  RFC 3526 for the group — are now here, with a section of their own
  below. What is not here is a modular exponentiation with a secret
  exponent: `Modulus::pow` in `crypto-bignum` takes an exponent of 64
  bits, and `pow_wide` says of itself that nothing in the product calls
  it and that it exists so a test can sign a certificate. That is the
  work the method waits on, not a document.
- **RFC 4419** is Diffie-Hellman group exchange, which RFC 9142 puts at
  SHOULD NOT in its SHA-1 form and MAY in its SHA-256 form. Nothing here
  negotiates a group.
- **RFC 7748 and RFC 8032** are Curve25519 and Ed25519, which RFC 8731
  and RFC 8709 defer to by name for the encodings, the aborts and the
  signing procedure. Both are now here, with the four other documents
  the cryptography of this repository was already written against, in
  the section below.

## The two documents of finite-field Diffie-Hellman

**RFC 3526** is six groups and nothing else: for each, the prime written
out in hexadecimal, the closed form it was derived from, the generator,
and the group's assigned id. Section 3 is group 14 — 2048 bits, generator
2, the prime `2^2048 - 2^1984 - 1 + 2^64 * { [2^1918 pi] + 124476 }` — and
it is the reason this file is kept rather than a constant with a comment.
Two thousand and forty-eight bits is sixty-four lines of hexadecimal that
no reader can check by eye and no derivation in this repository produces.

**RFC 8268** gives the group SSH names, and it does one more thing that
matters more than the names. Section 4 states that section 8 of RFC 4253
— which is in this directory, and which the Secure Shell section above
cites — contains an error. RFC 4253 writes the check on the peer's public
value as the closed interval `[1, p-1]`; RFC 8268 amends it to the open
one, `1 < e < p-1` and `1 < f < p-1`, and says why: the closed form
admits the values that force the shared secret into the two-element
subgroup. An implementation written from the text this repository holds,
without this one beside it, would accept a public value that makes the
key exchange meaningless. That is the case D-59 exists for: the document
is here so the correction cannot be missed, and RFC 4253 is kept as
published rather than quietly edited.

## The six documents the cryptography was already written against

These arrived together, and not because of any one protocol. Every one is
implemented in `crates/crypto` and cited there by section; none of them
had a copy in this directory, which meant a test could name a vector
whose source was not in the house. Loading Secure Shell's algorithm
documents made that visible: RFC 8731 and RFC 8709 defer to the first two
below by name, so the gap stopped being theoretical.

**RFC 7748** is Curve25519 and Curve448. Section 4.1 gives the curve,
section 5 the X25519 function with the clamping of the scalar, section
5.2 the test vectors `crypto-ec` is checked against, and section 6.1 the
Diffie-Hellman protocol built on it — including the all-zero check, with
the reason (a peer value of small order produces exactly that) and the
way to perform it without a side channel (or the bytes together). RFC
8731 makes that check a MUST for `curve25519-sha256`, and points here for
it.

**RFC 8032** is EdDSA. Sections 5.1.5, 5.1.6 and 5.1.7 are Ed25519's key
generation, signing and verification written as procedures, section 5.1.2
and 5.1.3 the encoding and decoding of a point, and section 7.1 the test
vectors. RFC 8709 is four lines of SSH framing around this document;
everything a signature actually is, is here.

**RFC 8439** is ChaCha20 and Poly1305, and it is kept for the density of
its vectors: a test vector for the quarter round, for the quarter round
on the state, for the block function, for the cipher, for Poly1305, and
for the key generation, each beside the algorithm it checks. Section 2.8
is the AEAD construction `crypto-aead` implements.

**RFC 2104** is HMAC: the construction with the two pads, and section 3,
which is the part that is got wrong — a key longer than the block is
hashed first, a shorter one is padded with zeros, and the block is the
hash function's block and not its output length.

**RFC 4231** is seven test cases for HMAC with the SHA-2 family, and it
is here for the three that are not the obvious one: a key longer than the
block, a key of one byte, and data longer than a block, which are the
cases where an implementation that skipped section 3 of RFC 2104 fails.

**RFC 5869** is HKDF: extract in section 2.2, expand in section 2.3, and
seven test cases in appendix A covering both hash functions and the
salt-less case. Sections 3.1 and 3.3 are the ones worth reading rather
than skipping — when a salt earns its place, and when the extract step
may be left out at all.

## Why RFC 8446

The P-384 and RSA sections above each left the same question open:
whether the TLS 1.3 specification belongs in this directory when only
pieces of it are cited. It does, and it is here now. `audhsos-tls`
implements this document, not a summary of it, and D-59 admits no
distinction between a standard that is implemented in part and one
implemented in whole. RFC 8448, which has been here from the beginning,
is a trace of *this* protocol; keeping the trace and not the
specification meant keeping the answers without the question.
