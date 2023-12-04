<?php
if(!$dev){
        function imgCall($Bpurl, $page, $imgsize, $imgcolor, $imgtype, $imgtime, $imgright){
            $Qimg[0] = '{"status":"error","data":{"error_code":24}}';

            $apis = array();
            if(!file_exists('disBing.txt')){$apis[] = 'https://api.qwant.com/v3/search/images/?count=75&offset=' . $page * 82 . '&uiv=1&locale=en_US&size=' . $imgsize . '&color=' . $imgcolor . '&imagetype=' . $imgtype . '&freshness=' . $imgtime . '&license=' . $imgright . '&q=' . $Bpurl;}
            if (!file_exists('disBing2.txt')) {$apis[] = 'https://librex.jojoyou3.repl.co/qwant.php?offset=' . $page * 82 . '&imgsize=' . $imgsize . '&imgcolor=' . $imgcolor . '&imgtype=' . $imgtype . '&imgtime=' . $imgtime . '&imgright=' . $imgright . '&q=' . $Bpurl;}
            
            $imgUrl='';
            if(count($apis)>1){$imgUrl = $apis[array_rand($apis)];}
            else{$imgUrl =  $apis[0];}

            $qCh = curl_init();
            curl_setopt($qCh, CURLOPT_URL, $imgUrl);
            curl_setopt($qCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
            curl_setopt($qCh, CURLOPT_CONNECTTIMEOUT, 2.5);
            curl_setopt($qCh, CURLOPT_RETURNTRANSFER, true);
            
            $qResponse = curl_exec($qCh);
            
            curl_close($qCh);

            $Qimg[0] = $qResponse;
            $Qimg[1] = $imgUrl;
            
            return $Qimg;
        }

        if(!isset($_SESSION[$Bpurl.$page.$imgsize.$imgcolor.$imgtype.$imgtime.$imgright.':-:imgBing'])){
        $Qimg = imgCall($Bpurl, $page, $imgsize, $imgcolor, $imgtype, $imgtime, $imgright);
        }
        else{
            $Qimg = $_SESSION[$Bpurl.$page.$imgsize.$imgcolor.$imgtype.$imgtime.$imgright.':-:imgBing'];
        }

if ($Qimg[0] == '{"status":"error","data":{"error_code":24}}' || $Qimg[0] == '{"status":"error","data":{"error_code":20}}') {
    if (!file_exists('disBing.txt') && parse_url($Qimg[1])['host'] == 'api.qwant.com') {file_put_contents('disBing.txt', time());}
    if (!file_exists('disBing2.txt') && parse_url($Qimg[1])['host'] == 'librex.jojoyou3.repl.co') {file_put_contents('disBing2.txt', time());}
    
    $Qimg = imgCall($Bpurl, $page, $imgsize, $imgcolor, $imgtype, $imgtime, $imgright);
}
        }
    else{
        $Qimg = file_get_contents('./Controller/dev/img.json');
    }
    if(!isset($_SESSION[$Bpurl.$page.$imgsize.$imgcolor.$imgtype.$imgtime.$imgright.':-:imgBing'])){
        $_SESSION[$Bpurl.$page.$imgsize.$imgcolor.$imgtype.$imgtime.$imgright.':-:imgBing'] = $Qimg[0];
        $Qimg = json_decode($Qimg[0], true);
    }
    else{
        $Qimg = json_decode($Qimg, true);
    }
        
        echo '<div style="display:flex;margin-top:30px;flex-wrap:wrap;justify-content:center;"><br>';
        foreach ($Qimg['data']['result']['items'] as &$item) {
            if (!isset($item['media']) or !isset($item['media_preview'])) {
                continue;
            }
    
            echo '
           <div class="imgoutdiv">
            <div tabindex="0" class="imgoutbtn">
                <img src="Controller/functions/proxy.php?q=http',urldecode(str_replace('&q=0&b=1&p=0&a=0','', explode('?u=http',$item['thumbnail'])[1])), '" class="imgout">
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
            <img src ="Controller/functions/proxy.php?q=http',urldecode(str_replace('&q=0&b=1&p=0&a=0','', explode('?u=http',$item['thumbnail'])[1])), '" data-src="/Controller/functions/proxy.php?q=',$item['media'],'"';
            if(!isset($_COOKIE['DisHImg'])){echo 'style="filter: blur(5px);"';}
            echo '>
            <br>
            <h3>', $item['title'], '</h3><br>
            <p>From website: ', $item['url'], '</p><br>
            <div class="bigimgbtn"><a href="', $item['url'], '"><button class="imgtoolsOption">Go to website</button></a><br>
            <a href="',$item['media'], '"> <button class="imgtoolsOption">Go to image</button></a></div>  
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
echo nextPage($purl, $page, $imgsize, $imgcolor,$imgtype, $imgtime, $imgright);
