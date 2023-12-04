<?php
$dev = true;
include 'Model/style.php';
include 'Controller/database.php';

if(isset($_POST['fedsubmit'])){ 
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
              mail('team@jojoyou.org', 'PriEco Feedback', 'Email: ' . $_POST['fedEmail'] . '
Feedback: ' . $_POST['fedsug']);
              
            echo '<h1>Thank you for your feedback!';
            header('refresh:2;url=/');
                exit();
        }
}
}
?>

<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta http-equiv="X-UA-Compatible" content="IE=edge">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Feedback | PriEco</title>
</head>
<body>
<div style="
    width: 100vw;
    height: 100vh;
    display: flex;
    justify-content: center;
    align-items: center;
">
<form method="POST" action="" style="
padding: 20px;
border: solid 7px gray;
border-radius: 20px;
">
<h4 style="font-size:32px;">Thank you for helping improve PriEco</h4>
<input type="text" name="fedEmail" placeholder="Email" class="fedimp" style="border-radius: 20px;
padding: 5px;
width: 100%;
height: 50px;
border: none;">
<p style="font-style: italic;font-size: 12px;"><b>Not required</b> just if you want us to contact you back</p>
<br><textarea name="fedsug" style="border-radius: 20px;
border: none;padding: 10px;
width: 100%;
height: 100px;margin-bottom:7px;" placeholder="Feedback*" class="fedimp" required></textarea>

<?php
 echo '<div class="h-captcha" data-sitekey="',$_ENV['hCaptcha_Site'],'"></div>';
if(isset($_POST['fedsubmit'])){ 
 if(empty($_POST['h-captcha-response'])){ 
     echo '<p style="color:red;margin:0;">Fill up hCaptcha!</p>';
 }
}
?>
<div style="text-align:center;">
<input type="submit" name="fedsubmit"value="Submit" style="border: solid 2px lightgray;
padding: 10px;
border-radius: 20px;
cursor: pointer;
font-weight: normal;display:inline;
margin-top: 10px;">
</form>
</div>
<br>
  </div>

  <script src="https://hcaptcha.com/1/api.js" async defer></script>

</body>
</html>