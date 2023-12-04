<?php
$imgUrl='';
if(!file_exists('disOpenVerse.txt')){$imgUrl = 'https://api.openverse.engineering/v1/images/?format=json&page_size=75&q=' . $Bpurl;}

    //Request new token
    if($_ENV['OPENVERSE']==''|| explode(';',$_ENV['OPENVERSE'])[1] <= time()){
        $curl = curl_init();
        curl_setopt_array($curl, array(
            CURLOPT_URL => "https://api.openverse.engineering/v1/auth_tokens/token/",
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_ENCODING => "",
            CURLOPT_MAXREDIRS => 10,
            CURLOPT_TIMEOUT => 0,
            CURLOPT_FOLLOWLOCATION => true,
            CURLOPT_HTTP_VERSION => CURL_HTTP_VERSION_1_1,
            CURLOPT_CUSTOMREQUEST => "POST",
            CURLOPT_POSTFIELDS => "grant_type=client_credentials&client_id=".$_ENV['OPENVERSE_CLIENT_ID']."&client_secret=".$_ENV['OPENVERSE_CLIENT_SECRET'],
            CURLOPT_HTTPHEADER => array(
                "Content-Type: application/x-www-form-urlencoded"
            ),
        ));
        $response = json_decode(curl_exec($curl),true);
        curl_close($curl);

        $envFile = explode("\n",file_get_contents('Controller/.env'));
        $envFile[count($envFile)-1]='OPENVERSE='.$response['access_token'].';'.time() + $response['expires_in'];

        file_put_contents('Controller/.env', implode("\n",$envFile));
        $_ENV['OPENVERSE'] = $response['access_token'];
    }


    if(!isset($_SESSION[$Bpurl.':-:imgOpen'])){
            $qCh = curl_init($imgUrl);
            curl_setopt($qCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
            curl_setopt($qCh, CURLOPT_CONNECTTIMEOUT, 2.5);
            curl_setopt($qCh, CURLOPT_RETURNTRANSFER, true);
            curl_setopt($qCh, CURLOPT_HTTPHEADER, array(
                "Authorization: Bearer ".explode(';',$_ENV['OPENVERSE'])[0],
              ));

            $openVerse = json_decode(curl_exec($qCh),true);
            if(isset($openVerse['results'])){
                $_SESSION[$Bpurl.':-:imgOpen']=json_encode($openVerse);
            }
            curl_close($qCh);
    }
    else{$openVerse = json_decode($_SESSION[$Bpurl.':-:imgOpen'],true);}
    
        echo '<div style="display:flex;margin-top:30px;flex-wrap:wrap;justify-content:center;"><br>';
        foreach ($openVerse['results'] as &$item) {
    
            echo '
           <div class="imgoutdiv">
            <div tabindex="0" class="imgoutbtn">
                <img src="Controller/functions/proxy.php?q=',$item['thumbnail'],'" class="imgout">
                <a style="color: var(--linkColor); cursor:pointer;text-decoration:none;" href="',$item['url'],'"';if (isset($_COOKIE['new'])) {echo 'target="_blank';}echo'>
            <p>';
                $pieces = parse_url($item['url']);
                $domain = isset($pieces['host']) ? $pieces['host'] : $pieces['path'];
                if (preg_match('/(?P<domain>[a-z0-9][a-z0-9\-]{1,63}\.[a-z\.]{2,6})$/i', $domain, $regs)) {
                    echo $regs['domain'];
                }
                echo '</p></a>
                <p>',$item['title'],'</p>
            </div>
            
            
    
            <div class="bigimgout">           
            <img src ="Controller/functions/proxy.php?q=',$item['thumbnail'],'" data-src="/Controller/functions/proxy.php?q=',$item['url'],'"';
            if(!isset($_COOKIE['DisHImg'])){echo 'style="filter: blur(5px);"';}
            echo '>
            <br>
            <h3>', $item['title'], '</h3><br>
            <p>From website: ', $item['url'], '</p><br>
            <div class="bigimgbtn"><a href="', $item['url'], '"><button class="imgtoolsOption">Go to website</button></a><br>
            <a href="',$item['url'], '"> <button class="imgtoolsOption">Go to image</button></a></div>  
            <div style="display: flex;justify-content: center;"><button class="bigimgclose imgtoolsOption">«Close</button></div>   
            </div>      
            </div>
            ';
        }
        echo '</div></div>';
   
    if(!isset($_COOKIE['DisHImg'])){
    echo '<script>
    const lazyImages = document.querySelectorAll("img[data-src]");

    const observer = new IntersectionObserver((entries, observer) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          const lazyImage = entry.target;
    
          lazyImage.src = lazyImage.dataset.src;
    
          lazyImage.removeAttribute("data-src");
    
          observer.unobserve(lazyImage);
    
          lazyImage.addEventListener("load", () => {
            lazyImage.classList.add("bigimage_loaded");
          });
        }
      });
    });
    
    // Start observing each lazy image
    lazyImages.forEach((lazyImage) => {
      observer.observe(lazyImage);
    });
    </script>';
}