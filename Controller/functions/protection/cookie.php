<?php
$num=0;
if(isset($_COOKIE['searchNum'])){
    $num = $_COOKIE['searchNum']+1;
}

setcookie('searchNum', $num, time() + 4, '/');

if($num >= 50){

    setcookie('searchNum', $num, time() + 21600, '/');

    if(isset($_POST['stop'])){ 
        setcookie('searchNum', 47, time() + 3600, '/');
        header("Refresh:0");
        exit();
    }
    else{
    echo'
    You have been blocked!
    <form method="post" action="">
        <input type="submit" name="stop" value="Stop">
    </form>
    ';
    header('refresh:2;url=http://example.com');
    exit();
    }
}
elseif($num >= 25){
    setcookie('searchNum', $num, time() + 3600, '/');

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
                setcookie('searchNum', $num, time() - 1, '/');
                header('refresh:0');
            }
} }
     echo'
    <div style="width: 100vw;
    text-align: center;
    padding-top: 20vh;">
        <h1 style="filter: drop-shadow(0 0 10px red);">Shield 🛡️</h1>
        <p><b><h3 style="display:inline;">You have been blocked! </h3></b>We are sorry but we had to block you from searching. 
        You have been searching really fast and we had to block you to protect PriEco.
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
    ';
    die();
}
elseif($num >= 10){

    echo '
    <div style="width: 100vw;
    text-align: center;
    padding-top: 20vh;">
    <h3>Calm down! You are searching too fast!</h3><br>
    <p style="display:inline;">Wait </p>
  <div class="digit">
    <span class="seconds-digit-two">0</span>
    <span>5</span>
    <span>4</span>
    <span>3</span>
    <span>2</span>
    <span>1</span>
    <span>0</span>
  </div>

<p style="display:inline;"> seconds.</p>
</div>
<style>

@keyframes tick10 {
    0%      { margin-top: 0; }
    16%     { margin-top: -2rem;  }
    32%     { margin-top: -4rem;  }
    48%     { margin-top: -6rem;  }
    64%     { margin-top: -8rem;  }
    80%     { margin-top: -10rem;  }
    100%     { margin-top: -12rem;  }
}


.digit {
display: inline-block;
height: 1rem;
overflow: hidden;
width: 1ch;
}

.digit span {
display: block;
height: 2rem;
width: 100%;
}

.seconds-digit-two {
animation: tick10 6s;
}</style>';
header('refresh:6');
exit();
}
