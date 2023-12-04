<!--  SPDX-FileCopyrightText: 2022, 2022-2022 Roman  Láncoš <jojoyou@jojoyou.org> -->
<!-- -->
<!--  SPDX-License-Identifier: AGPL-3.0-or-later -->

<!DOCTYPE html>
<html lang="en">
<head>
  <title><?php if($purl != '' && $purl != null && !isset($_COOKIE['hQuery'])){echo str_replace('q=','',str_replace('&q=','',urldecode($purl))),' | PriEco';}else{echo'PriEco';}?></title>
  <meta name="description" content="PriEco, the Private, Secure and Ecofriendly search engine.">
  <meta charset="UTF-8">
  <meta http-equiv="X-UA-Compatible" content="IE=edge">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta http-equiv="X-XSS-Protection" content="1; mode=block">
  <meta http-equiv="Content-Security-Policy" content="
    default-src 'self';
    script-src 'self' 'unsafe-inline' https://unpkg.com;
    style-src 'self' 'unsafe-inline' https://unpkg.com;
    img-src 'self' data:;
    connect-src 'self' https://nominatim.openstreetmap.org;
    frame-src 'self';
    object-src 'none';
    child-src https://tile.openstreetmap.org;">

  <link rel="icon" href="./favicon.ico?1">
  <link rel="search"
      type="application/opensearchdescription+xml"
      title="PriEco"
      href="osd.xml">
</head>

<body>