<?php
$ip = hash('sha256',$_SERVER['REMOTE_ADDR']);
$attackid = hash('sha256',$_SERVER['REMOTE_ADDR'].$_SERVER['HTTP_USER_AGENT']);
$tmp = date('H');

$wasnt = false;

$sql = "SELECT * FROM `ip` WHERE `ip` = '$ip'";
$result = $conn->query($sql);

$ipsearches = $result->fetch_assoc();
$ipusr = explode(' ',$ipsearches['devices']);
$ipallow = explode(' ', $ipsearches['allow']);
$ipcount = count($ipusr)-1;

if($ipcount == 0){
    $sql = "DELETE FROM `ip` WHERE `date` != '$tmp'";
    $conn->query($sql);
    $sql = "INSERT INTO `ip` (`ip`, `devices`, `searches`, `date`) VALUES ('$ip','$attackid',0,$tmp)";
    $conn->query($sql);
    $wasnt = true;
}

$ipdate = $ipsearches['date'];
$ipsearches = $ipsearches['searches']+1;

if(!in_array($attackid, $ipusr)){
    $tmp = implode(' ',$ipusr).' '.$attackid;
    $sql = "UPDATE `ip` SET `devices` = '$tmp' WHERE `ip` = '$ip';";
    $conn->query($sql);

    ++$ipcount;
}
if(!in_array($attackid, $ipallow)){
$sql = "UPDATE `ip` SET `searches` = '$ipsearches' WHERE `ip` = '$ip';";
$conn->query($sql);
}
if($ipsearches >= $ipcount*40 && !in_array($attackid, $ipallow) && !$wasnt){

if(isset($_POST['submit'])){ 
    if(!empty($_POST['h-captcha-response'])){ 
        $verifyURL = 'https://hcaptcha.com/siteverify'; 
        $token = $_POST['h-captcha-response']; 
        $data = array( 
            'secret' => $_ENV['hCaptcha_Secret'], 
            'response' => $token, 
            'remoteip' => $_SERVER['REMOTE_ADDR'] 
        ); 
        $curlConfig = array( 
            CURLOPT_URL => $verifyURL, 
            CURLOPT_POST => true, 
            CURLOPT_RETURNTRANSFER => true, 
            CURLOPT_POSTFIELDS => $data 
        ); 
        $ch = curl_init(); 
        curl_setopt_array($ch, $curlConfig); 
        $response = curl_exec($ch); 
        curl_close($ch); 
        $responseData = json_decode($response); 
        if($responseData->success){ 
            $tmp = implode(' ',$ipallow).' '.$attackid;
            $sql = "UPDATE `ip` SET `allow`='$tmp' WHERE `ip` = '$ip'";
            $conn->query($sql);
            header('refresh:0');
        }
} }
 echo'
 <!--  SPDX-FileCopyrightText: 2022, 2022-2022 Roman  Láncoš <jojoyou@jojoyou.org> -->
<!-- -->
<!--  SPDX-License-Identifier: AGPL-3.0-or-later -->

<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="X-UA-Compatible" content="IE=edge">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">

  <link rel="icon" href="./favicon.ico?1">
  <link rel="search"
      type="application/opensearchdescription+xml"
      title="PriEco"
      href="osd.xml">
</head>

<body>
<div style="width: 100vw;
text-align: center;
padding-top: 20vh;">
    <h1 style="filter: drop-shadow(0 0 10px red);">IP Shield 🛡️</h1>
    <p><b><h3 style="display:inline;">Your IP address has been blocked! </h3></b>We are sorry but we had to block you from searching. 
    Your IP address (e.g. house, work...) has been searching too fast and we had to block you to protect PriEco.
    <br><b>If you aren`t bot or attacker please fill this CAPTCHA.</b></p>

    <form action="" method="post">
        <div class="h-captcha" data-sitekey="',$_ENV['hCaptcha_Site'],'"></div>
        ';
    if(isset($_POST['submit'])){ 
        if(empty($_POST['h-captcha-response'])){ 
            echo '<p style="color:red;margin:0;">Fill up hCaptcha!</p>';
        }
    }
    echo'
        <input type="submit" name="submit" value="🔓 Unlock" style="background-color: #e7e7e7;
        border: none;
        border-radius: 20px;
        font-weight: bold;
        cursor: pointer;
        padding: 5px;">
    </form>
    
</div>

<script src="https://hcaptcha.com/1/api.js" async defer></script>
</body>
';
die();

}