<!--  SPDX-FileCopyrightText: 2022, 2022-2022 Roman  Láncoš <jojoyou@jojoyou.org> -->
<!-- -->
<!--  SPDX-License-Identifier: AGPL-3.0-or-later -->

<?php

if(!isset($_COOKIE['DisSugges'])) {
  include './View/html/suggest.php';

}
if(!isset($_COOKIE['DisQue'])) {
include './View/html/delQuery.html';
}
?>

<div class="footer"> 
<?php
if(isset($_COOKIE['datavase'])){
  echo '
<img src="./View/img/PriEco.webp" style="width: 50px;
position: absolute;
left: 200px;
z-index: 5;
margin-top: -70px;" alt="bottomTree">
<img src="./View/img/PriEco.webp" style="  width: 45px;
  position: absolute;
  left: 170px;
  z-index: 3;
  margin-top: -70px;"alt="bottomTree">
<img src="./View/img/PriEco.webp" style="width: 40px;
position: absolute;
left: 235px;
z-index: 3;
margin-top: -63px;"alt="bottomTree">
';
}
?> 
<div class="footer-content">

    <div style="float:left;padding-left: 20px;width: 25%;word-wrap: break-word;" class="footer-content-left">
      <p>
        <br>
        Privacy Policy and TOS: <a href="https://jojoyou.org/legal/" target="_blank" style="text-decoration:none;color:var(--linkColor);">Legal</a>
          <br>
          Source code: <a href="https://codeberg.org/JojoYou/PriEco/src/branch/master" target="_blank" style="text-decoration:none;color:var(--linkColor);">CodeBerg</a>
          <br>
          
          Tor onion domain: <a href="http://priecovk7jsuh3tvkh62c6j4oep3l5bldigpzmay26rdpqz357t5dmad.onion/" target="_blank" style="text-decoration:none;color:var(--linkColor);">Onion</a>
          <br><br>
        </p>
    </div>
    <div style="text-align: center;width: 50%;float: left;" class="footer-content-center">
      <h2 style="background: -webkit-linear-gradient(#03D781, #3EDCE2);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;font-size: 30px;
font-weight: bold;">
        PriEco
      </h2>
  <p>1.0.1</p>
  
  <h3><span style="display:inline-block;
  transform: rotate(180deg);">&copy;</span>2021-<?php echo date("Y"); ?> Roman Láncoš, JojoYou</h3>
  <a href="https://codeberg.org/JojoYou/PriEco/src/branch/master/LICENCE.md" target="_blank"><button style="background: none;
border: 3px solid #28dabe;
border-radius: 7px;
width: auto;
height: auto;
cursor: pointer;
padding:2px;
" >See License</button></a>
    </div>
    <div style="width: 25%;float:right;text-align: right;padding-right: 10px;word-wrap: break-word;" class="footer-content-right">
      <p>
          <b>Donate</b>
          <br>
          <p style="display:inline;">BTC </p><label class="footerDonateButton" for="footerDonateOpen1">V</label>
          <input type="checkbox" class="footerDonateOpen" id="footerDonateOpen1" style=><p class="footerDonateHide" id="footerDonateHide1">18CMebkyUdamDL555ymSq9DAnCGY419Unu</p>
<br>
          <p style="display:inline;">ETH </p><label class="footerDonateButton" for="footerDonateOpen2">V</label>
          <input type="checkbox" class="footerDonateOpen" id="footerDonateOpen2" style=><p class="footerDonateHide" id="footerDonateHide2">0x78772e0a9D806Bfd2594E7e3317429dDf81f8946</p>
<br>
<p style="display:inline;">SOL </p><label class="footerDonateButton" for="footerDonateOpen3">V</label>
          <input type="checkbox" class="footerDonateOpen" id="footerDonateOpen3" style=><p class="footerDonateHide" id="footerDonateHide3">HX3tkpjgZbNT9VBZN2XysvMsW2JEAGTa7DVpxWSRUGDR</p>
<br>
<p style="display:inline;">Monero </p><label class="footerDonateButton" for="footerDonateOpen4">V</label>
          <input type="checkbox" class="footerDonateOpen" id="footerDonateOpen4" style=><p class="footerDonateHide" id="footerDonateHide4">43rUq5xvwUh9MaUobAD7Pw3zUtasuUE9Z2nqZaLK2fyZLa1sXpto4xh2fV7sYDGq8UDDrnN3KCKB9AaynWwwESdnCbRrotG</p>
        </p>
    </div>
  </div>
</div>