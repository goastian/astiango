<?php

function shield($conn,$purl){
#hCaptcha
if(isset($_SESSION['Pass'])){return;}

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
           $_SESSION['Pass'] = true;
            header('refresh:0');
        }
}
}

$pass = true;
#IP
if(!isset($_SESSION['IPPass'])){
$ip = $_SERVER['REMOTE_ADDR'];
$sql = "SELECT * FROM `ipblock` WHERE MATCH(ip) AGAINST(INET_ATON('$ip') IN BOOLEAN MODE) LIMIT 1;";
$result = mysqli_query($conn, $sql);
if (mysqli_num_rows($result) > 0) {$pass=false;}
else{$_SESSION['IPPass']=true;}
}
#Suspicious words
$words = ['slot'];
if (!empty(array_intersect(explode(' ', $purl), $words))) {$pass = false;}


#CAPTCHA
if(!$pass){
    header('HTTP/1.0 403 Forbidden');
    echo'<!DOCTYPE html><html lang="en"><head><title>Blocked | PriEco</title><meta name="description" content="PriEco, the Private, Secure and Ecofriendly search engine."><meta charset="UTF-8"><meta http-equiv="X-UA-Compatible" content="IE=edge"><meta name="viewport" content="width=device-width, initial-scale=1.0"><link rel="icon" href="./favicon.ico?1"><link rel="search"type="application/opensearchdescription+xml"title="PriEco"href="osd.xml"></head><body><div style="width: 100vw;text-align: center;padding-top: 20vh;"><h1 style="filter: drop-shadow(0 0 10px red);">Shield 🛡️</h1><p><b>Your activity looks suspicious to us.</b><br>It is alright, if you are a real human, fill up this CAPTCHA and click unlock.</p><form action="" method="post"><div class="h-captcha" data-sitekey="',$_ENV['hCaptcha_Site'],'"></div>';if(isset($_POST['submit'])){if(empty($_POST['h-captcha-response'])){echo '<p style="color:red;:0;Fill up hCaptcha!</p>';}}echo'<input type="submit" name="submit" value="🔓 Unlock" style="background-color: #e7e7e7;border: none;border-radius: 20px;font-weight: bold;cursor: pointer;padding: 5px;"></form></div><script src="https://hcaptcha.com/1/api.js" async defer></script></body></html>';
    exit();
}
return;
}
shield($conn,$purl);