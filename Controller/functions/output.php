<!--  SPDX-FileCopyrightText: 2022, 2022-2022 Roman  Láncoš <jojoyou@jojoyou.org> -->
<!-- -->
<!--  SPDX-License-Identifier: AGPL-3.0-or-later -->

<?php
function getSearchQueryComplexity($purl) {
    $words = explode(' ', $purl);
    $uniqueWords = count($words);
    $averageWordLength = array_sum(array_map('strlen', $words)) / $uniqueWords;
    $specialCharacterCount = 1;
    foreach ($words as $word) {$specialCharacterCount += (strpos($word, '[^a-zA-Z0-9]') !== false);}
    return $complexityScore = $uniqueWords * $averageWordLength * $specialCharacterCount;  
  }
function apiRandom($apis = array()){
    shuffle($apis);
    return $apis[0];
}
function chooseAPI($disabled, &$complex){
    if($complex >= 20){
        $apis = [];
        if($disabled[0]==0){$apis[]=0;}
        if($disabled[1]==0){$apis[]=1;}
        
        if(empty($apis)){$complex = 19;}
        else{$searchId = apiRandom($apis);}
    }
    
    if($complex >= 10 && $complex < 20){
        $apis = [];
        if($disabled[1]==0){$apis[]=1;}
        if($disabled[2]==0){$apis[]=2;}
    
        if(empty($apis)){$complex = 9;}
        else{$searchId = apiRandom($apis);}
    }
    if($complex >= 10 && $complex < 10){
        $apis = [];
        if($disabled[2]==0){$apis[]=2;}
        if($disabled[3]==0){$apis[]=3;}
        
        if(empty($apis)){$complex = 9;}
        else{$searchId = apiRandom($apis);}
    }
    if($complex < 10){
        $apis = [];
        if($disabled[3]==0){$apis[]=3;}
        if($disabled[4]==0){$apis[]=4;}
        
        if(empty($apis)){$complex = 5;}
        $searchId = apiRandom($apis);
    }
    if($complex <= 5){
        if($disabled[4]==0){$apis[]=4;}
        if($disabled[5]==0){$apis[]=5;}

        $searchId = apiRandom($apis);
    }
    return $searchId;
}
function backupCall($searchId, $disabled, &$complex, $Bpurl, $MojeekFile, $lang){
    if($complex >= 30){$complex=29;}
    elseif($complex >= 20){$complex=19;}
    else{$complex=9;}
    $searchId = chooseAPI($disabled, $complex);

    $ch3 = curl_init();

    if($searchId==1){
        $ch3 = curl_init('https://librex.jojoyou3.repl.co/api.php?p=0&t=0&q='.$Bpurl);
        $tmp = $lang;
        if($lang = 'all'){$tmp = 'en';}
        $cookies = 'google_language_results='.$tmp.';google_number_of_results=20;google_language_site='.$tmp.';';
        if(!isset($_COOKIE['safe'])){$cookies.='safe_search=on;';} 
        curl_setopt($ch3, CURLOPT_COOKIE, $cookies);
    }
    elseif($searchId==2){
        $ch3 = curl_init('https://api.qwant.com/v3/search/web/?count=10&offset='.$page.'0&uiv=1&locale=en_us&q=' . $Bpurl);
        curl_setopt($ch3, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    }
    elseif($searchId==3){
        $ch3 = curl_init('https://bing2.jojoyou3.repl.co/?q=' . $Bpurl);
        curl_setopt($ch3, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    }
    elseif($searchId==4){
        $ch3 = curl_init('https://search.brave.com/search?q='.$Bpurl);
        curl_setopt($ch3, CURLOPT_CUSTOMREQUEST, 'GET');
        $headers = array(
            'Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8',
            'Accept-Language: en-US,en;q=0.8',
            'Cache-Control: max-age=0',
            'Connection: keep-alive',
            'Upgrade-Insecure-Requests: 1',
            'User-Agent: Mozilla/5.0 (Windows NT 6.1; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/56.0.2924.87 Safari/537.36',
        );
        curl_setopt($ch3, CURLOPT_HTTPHEADER, $headers);
    }
    elseif($searchId==5){$ch3 = curl_init($MojeekFile);}

    curl_setopt($ch3, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($ch3, CURLOPT_CONNECTTIMEOUT, 2.5);

    if($searchId == 4){$response = curl_exec($ch3);}
    else{$response = json_decode(curl_exec($ch3),true);}
    
    curl_close($ch3);
    return [$response, $searchId];
}  
  
$Bpurl = urlencode($purl);

if ($type != 'image' && $type != 'video' && $type != 'news' && $type != 'shop') {
    //Get string between characters
    function get_string_betweens($string, $start, $end)
    {
        $string = ' ' . $string;
        $ini = strpos($string, $start);
        if ($ini === false) {return '';}
        $ini += strlen($start);
        $len = strpos($string, $end, $ini);
        if ($len === false || $len <= $ini) {return '';}
        $len -= $ini;
        if ($len < 0) {return '';}
        
        return substr($string, $ini, $len);
    }
    
    
    //Initial call
    $indexY = true;
    
    if(!$dev){
        $ImpProfiles = false;
        $ImpGoogle = true;
        $name = mysqli_real_escape_string($conn, strtolower($purl));  
        if(isset($_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']])){

            if (strpos($_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']], 'obj +=+ ') !== false) {$obj = json_decode(str_replace('obj +=+ ', '', $_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']]), true);}
            if (strpos($_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']], 'g2obj +=+ ') !== false) {$g2obj = json_decode(str_replace('g2obj +=+ ', '', $_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']]), true);}
            if (strpos($_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']], 'qwant +=+ ') !== false) {$QWantObj = json_decode(str_replace('qwant +=+ ', '', $_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']]), true);}
            if (strpos($_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']], 'brave +=+ ') !== false) {$BraveObj = str_replace('brave +=+ ', '', $_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']]);}
            if (strpos($_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']], 'mojeek +=+ ') !== false) {$MojeekObj = json_decode(str_replace('mojeek +=+ ', '', $_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']]), true);}
            
            if(isset($_SESSION[$purl.':-:shop'])){$ShopObj = json_decode($_SESSION[$purl.':-:shop'],true);}
            if(isset($_SESSION[$purl.':-:wiki'])){$wikiobj = explode('--]|[--',$_SESSION[$purl.':-:wiki']);$wikiTxt=$wikiobj[1];$ddgObj = json_decode($wikiobj[2],true);$wikiobj=json_decode($wikiobj[0],true);}
            if(isset($_SESSION[$purl.':-:new'])){$NewsObj = $_SESSION[$purl.':-:new'];}
            if(isset($_SESSION[$purl.':-:red'])){$redditObj = $_SESSION[$purl.':-:red'];}
            if(isset($_SESSION[$purl.':-:yt'])){$YoutubeObj = $_SESSION[$purl.':-:yt'];}
            if(isset($_SESSION[$purl.':-:rel'])){$related = $_SESSION[$purl.':-:rel'];}

            if (preg_match('/\bip\b/i', $purl)) {
                $ipCh = curl_init('https://jojoyou.org/ipAPI/?ip='.$_SERVER['REMOTE_ADDR']);
                curl_setopt($ipCh, CURLOPT_RETURNTRANSFER, true);
                curl_setopt($ipCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
                curl_setopt($ipCh, CURLOPT_CONNECTTIMEOUT, 2);
                curl_setopt($ipCh, CURLOPT_TIMEOUT, 2.5);
                $ipObj = json_decode(curl_exec($ipCh),true);        
                curl_close($ipCh);
            }
            $cached = true;
            $ImpGoogle = false;
        }
        else{
        //Cached Google results
      
        $obj = '';
        if(!isset($_COOKIE['safe']) && !isset($_COOKIE['time'])){
        if(!isset($loc)){
            $loc = 'all';
        }
        if(!isset($lang)){
            $lang = 'all';
        }
        $sql = "SELECT * FROM `googleCache` WHERE `query` = '$name' AND `loc` = '$loc' AND `lang` = '$lang'";
        $tmp = $conn->query($sql);
        if ($tmp !== false) {
        $tmp = $tmp->fetch_assoc();
        }

        $obj = isset($tmp['results']) ? $tmp['results'] : '';

        if($obj != ''){
            if($tmp['official'] == 1){
            $ImpGoogle = false;
            $obj = json_decode($obj, true);
            $searchId = 0;
            }
            else{
                $ImpGoogle = false;
                $g2obj = json_decode($obj, true);
                $obj = '';
                $searchId=1;
            }
        }
        }

        //Wiki
    $name = str_replace('+',' ',urlencode(ucwords($purl)));
    $result = $conn->query("SELECT * FROM `wikipedia` WHERE '$name' LIKE CONCAT(title, '%');");
    $indexW = false;
    if ($result->num_rows > 0) {
        $closestTitle = null;
        $closestDistance = PHP_INT_MAX;

        while ($row = $result->fetch_assoc()) {
            $title = $row['title'];
            $distance = levenshtein($name, $title);

            if ($distance < $closestDistance && $distance < 5) {
                $closestDistance = $distance;
                $closestTitle = $title;
                $wikiobj = json_decode($row['infobox'], true);
                $wikiTxt = $row['paragraph'];
                $ddgObj = $row['profiles'];
                $indexW = true;
            }
        }
    }
    
     //Reddit
     $redditObj = array();
     $result = $conn->query("SELECT * FROM `reddit` WHERE `title` LIKE '%$purl%' LIMIT 200");
     if ($result->num_rows > 0) {
         while ($row = $result->fetch_assoc()) {
             $redditObj[] = $row;
         }
     }

     //News
     $NewsObj = array();
     $result = $conn->query("SELECT * FROM `news` WHERE `title` LIKE '%$purl%' OR `description` LIKE '%$purl%' LIMIT 200");
     if ($result->num_rows > 0) {
         while ($row = $result->fetch_assoc()) {
             $NewsObj[] = $row;
         }
     }
     usort($NewsObj, function ($a, $b) {
         $timestampA = strtotime($a['date']);
         $timestampB = strtotime($b['date']);
     
         if ($timestampA === $timestampB) {return 0;}
         return ($timestampA > $timestampB) ? -1 : 1;
     });
     if(count($NewsObj)<3){$indexN = false;}
     else{$indexN = true;}

      //YouTube
    $YoutubeObj = array();
    $result = $conn->query("SELECT * FROM `youtube` WHERE `title` LIKE '%$purl%' OR `description` LIKE '%$purl%' LIMIT 200");
    if ($result->num_rows > 0) {
        while ($row = $result->fetch_assoc()) {
            $YoutubeObj[] = $row;
        }
    }
    usort($YoutubeObj, function ($a, $b) {
        $timestampA = strtotime($a['date']);
        $timestampB = strtotime($b['date']);
    
        if ($timestampA === $timestampB) {return 0;}
        return ($timestampA > $timestampB) ? -1 : 1;
    });
    if(count($YoutubeObj)<3){$indexY = false;unset($YoutubeObj);}
    
    //Related
    $result = $conn->query("SELECT * FROM `suggestions` WHERE MATCH(name) AGAINST('$purl*') LIMIT 200");

$suggestions = [];
if ($result->num_rows > 0) {
    while ($row = $result->fetch_assoc()) {
        $suggestions[] = [
            'name' => $row['name'],
            'img' => $row['img']
        ];
    }
}

$similarityScores = [];
foreach ($suggestions as $suggestion) {
    if (substr($suggestion['name'], 0, 1) === "!" && substr($purl, 0, 1) !== "!") {
        continue;
    }
    if (strlen($suggestion['name']) < strlen($purl)) {
        continue;
    }

    $similarityScores[$suggestion['name']] = similar_text($purl, $suggestion['name']) + (10 - strlen($suggestion['name']));
}

arsort($similarityScores);

$topSuggestions = array_keys(array_slice($similarityScores, 0, 6));
$related = [];
foreach ($topSuggestions as $suggestion) {
    foreach ($suggestions as $item) {
        if ($item['name'] === $suggestion) {
            $related[] = $item;
            break;
        }
    }
}
    }

if(!isset($cached)){
        // Initialize multi-curl handle
$multiHandle = curl_multi_init();

// Initialize curl handles for each request
$curlHandles = array();
$curlCount = 0;
$disabled =array_fill(0,6,1);
    $disabled[4]=0;

// Request 3: Googlefile, Brave or Qwant
if($obj == '' && ($g2obj == '' or $g2obj['status']=='error')){
    $complex = getSearchQueryComplexity($purl);
    $searchId = -1;
    
    $disabled =array_fill(0,6,0);
    if(file_exists('disGoogle.txt')){$disabled[0]=1;}
    if(file_exists('disGoogle2.txt')){$disabled[1]=1;}
    if(file_exists('disBing.txt')){$disabled[2]=1;}
    if(file_exists('disBing2.txt')){$disabled[3]=1;}
    if(file_exists('disBrave.txt')){$disabled[4]=1;}
    if(file_exists('disMojeek.txt')){$disabled[5]=1;}
    
    $searchId = chooseAPI($disabled,$complex);

    if($searchId==0){$ch3 = curl_init($Googlefile);}
    elseif($searchId==1){
        $ch3 = curl_init('https://librex.jojoyou3.repl.co/api.php?p=0&t=0&q='.$Bpurl);
        $tmp = $lang;
        if($lang = 'all'){$tmp = 'en';}
        $cookies = 'google_language_results='.$tmp.';google_number_of_results=20;google_language_site='.$tmp.';';
        if(!isset($_COOKIE['safe'])){$cookies.='safe_search=on;';} 
        curl_setopt($ch3, CURLOPT_COOKIE, $cookies);
    }
    elseif($searchId==2){
        $ch3 = curl_init('https://api.qwant.com/v3/search/web/?count=10&offset='.$page.'0&uiv=1&locale=en_us&q=' . $Bpurl);
        curl_setopt($ch3, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    }
    elseif($searchId==3){
        $ch3 = curl_init('https://bing2.jojoyou3.repl.co/?q=' . $Bpurl);
        curl_setopt($ch3, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    }
    elseif($searchId==4){
        $ch3 = curl_init('https://search.brave.com/search?q='.$Bpurl);
        curl_setopt($ch3, CURLOPT_CUSTOMREQUEST, 'GET');
        $headers = array(
            'Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8',
            'Accept-Language: en-US,en;q=0.8',
            'Cache-Control: max-age=0',
            'Connection: keep-alive',
            'Upgrade-Insecure-Requests: 1',
            'User-Agent: Mozilla/5.0 (Windows NT 6.1; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/56.0.2924.87 Safari/537.36',
        );
        curl_setopt($ch3, CURLOPT_HTTPHEADER, $headers);
        if($loc != 'all'){curl_setopt($ch3, CURLOPT_COOKIE, 'country='.$loc.';');}
    }
    elseif($searchId==5){
        $ch3 = curl_init($MojeekFile);
    }

    curl_setopt($ch3, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($ch3, CURLOPT_CONNECTTIMEOUT, 2.5);
    $curlHandles[] = $ch3;
    if(count($curlHandles) > $curlCount){
        $searchOn = true;
        $curlCount = count($curlHandles);
    }
}


$tmp = $_COOKIE['Language'];
    if($tmp == 'all' or $tmp == null){
        $tmp = 'en';
    }

//Define
if (isset($defWords)) {
    $defCh = curl_init($WordnikFile);
    curl_setopt($defCh, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($defCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    curl_setopt($defCh, CURLOPT_CONNECTTIMEOUT, 2);
    curl_setopt($defCh, CURLOPT_TIMEOUT, 2.5);

      $curlHandles[] = $defCh;
  if(count($curlHandles) > $curlCount){
    $defOn = true;
    $curlCount = count($curlHandles);
    }
}
//Weather
if ($weatherTrue) {
    $weatherCh = curl_init($OpenWeatherFile);
    curl_setopt($weatherCh, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($weatherCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    curl_setopt($weatherCh, CURLOPT_CONNECTTIMEOUT, 2);
    curl_setopt($weatherCh, CURLOPT_TIMEOUT, 2.5);

      $curlHandles[] = $weatherCh;
  if(count($curlHandles) > $curlCount){
    $weatherOn = true;
    $curlCount = count($curlHandles);
    }

    $weatherForecastCh = curl_init($OpenWeatherForecastFile);
    curl_setopt($weatherForecastCh, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($weatherForecastCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    curl_setopt($weatherForecastCh, CURLOPT_CONNECTTIMEOUT, 2);
    curl_setopt($weatherForecastCh, CURLOPT_TIMEOUT, 2.5);
      $curlHandles[] = $weatherForecastCh;
  if(count($curlHandles) > $curlCount){
    $weatherForecastOn = true;
    $curlCount = count($curlHandles);
    }
}

//IP
if (preg_match('/\bip\b/i', $purl)) {
    $ipCh = curl_init('https://jojoyou.org/ipAPI/?ip='.$_SERVER['REMOTE_ADDR']);
    curl_setopt($ipCh, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($ipCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
    curl_setopt($ipCh, CURLOPT_CONNECTTIMEOUT, 2);
    curl_setopt($ipCh, CURLOPT_TIMEOUT, 2.5);
      $curlHandles[] = $ipCh;
  if(count($curlHandles) > $curlCount){
    $ipOn = true;
    $curlCount = count($curlHandles);
    }
}
/*
//Crypto
$cryptoCh = curl_init('https://api.coingecko.com/api/v3/coins/'.$purl);
curl_setopt($cryptoCh, CURLOPT_RETURNTRANSFER, true);
curl_setopt($cryptoCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
curl_setopt($cryptoCh, CURLOPT_CONNECTTIMEOUT, 2);
$curlHandles[] = $cryptoCh;
if(count($curlHandles) > $curlCount){
    $cryptoOn = true;
    $curlCount = count($curlHandles);
}
*/

//Wikipedia
if(!$indexW){
$wikiCh = curl_init('https://search.jojoyou.org/Controller/functions/addons/wikiGet.php?lang='.$tmp.'&q=' .str_replace('+','_',urlencode(ucwords($purl))));
curl_setopt($wikiCh, CURLOPT_RETURNTRANSFER, true);
curl_setopt($wikiCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
curl_setopt($wikiCh, CURLOPT_CONNECTTIMEOUT, 2);
curl_setopt($wikiCh, CURLOPT_TIMEOUT, 2.5);

$curlHandles[] = $wikiCh;
if(count($curlHandles) > $curlCount){
    $wikiOn = true;
    $curlCount = count($curlHandles);
}
}
//News
if(!$indexN){
$newsCh = curl_init($NewsFile);
curl_setopt($newsCh, CURLOPT_RETURNTRANSFER, true);
curl_setopt($newsCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
curl_setopt($newsCh, CURLOPT_CONNECTTIMEOUT, 2);
curl_setopt($newsCh, CURLOPT_TIMEOUT, 2.5);
$curlHandles[] = $newsCh;
if(count($curlHandles) > $curlCount){
    $newsOn = true;
    $curlCount = count($curlHandles);
}
}
//YouTube
if(!$indexY){
$ytCh = curl_init($YoutubeFile);
curl_setopt($ytCh, CURLOPT_RETURNTRANSFER, true);
curl_setopt($ytCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
curl_setopt($ytCh, CURLOPT_CONNECTTIMEOUT, 2);
curl_setopt($ytCh, CURLOPT_TIMEOUT, 2.5);
$curlHandles[] = $ytCh;
if(count($curlHandles) > $curlCount){
    $ytOn = true;
    $curlCount = count($curlHandles);
}
}

//Shop Ads
$tmp = ($lang == 'all') ? 'us' : $loc;

$shopCh = curl_init('https://api.yadore.com/v2/offer?market='.$tmp.'&keyword='.urlencode($purl).'&precision=fuzzy&sort=rel_desc&limit=20');
    curl_setopt($shopCh, CURLOPT_RETURNTRANSFER, true);

    $headers = ['API-Key: '. $_ENV['SHOP_API_KEY'],];
    curl_setopt($shopCh, CURLOPT_HTTPHEADER, $headers);
    $curlHandles[] = $shopCh;
    if(count($curlHandles) > $curlCount){
        $shopOn = true;
        $curlCount = count($curlHandles);
    }
    
// Execute multi-curl requests
foreach ($curlHandles as $handle) {
    curl_multi_add_handle($multiHandle, $handle);
}

$running = null;
do {
    curl_multi_exec($multiHandle, $running);
    curl_multi_select($multiHandle);
} while ($running > 0);

// Close the multi-curl handle
curl_multi_close($multiHandle);

// Get response content and error codes for each request
foreach ($curlHandles as $handle) {
    if (curl_error($handle) === '') {
        $response = curl_multi_getcontent($handle);

        if(isset($searchOn) && $searchId == 0){$obj = json_decode($response, true);unset($searchOn);}
        elseif(isset($searchOn) && $searchId == 1){$g2obj = json_decode($response, true);unset($searchOn);}
        elseif(isset($searchOn) && $searchId == 2){$QWantObj = json_decode($response, true); unset($searchOn);}
        elseif(isset($searchOn) && $searchId==3){$QWantObj = json_decode($response, true); unset($searchOn);}
        elseif(isset($searchOn) && $searchId==4){$BraveObj = $response; unset($searchOn);}
        elseif(isset($searchOn) && $searchId==5){$MojeekObj = json_decode($response, true); unset($searchOn);}

        elseif(isset($defOn) && !isset($WordnikObj)){$WordnikObj = json_decode($response, true);unset($defOn);}
        elseif(isset($weatherOn) && !isset($OpenWeatherObj)){$OpenWeatherObj = json_decode($response, true);unset($weatherOn);}
        elseif(isset($weatherForecastOn) && !isset($OpenWeatherForecastObj)){$OpenWeatherForecastObj = json_decode($response, true);unset($weatherForecastOn);}
        elseif(isset($ipOn) && !isset($ipObj)){$ipObj = json_decode($response,true); unset($ipOn);}
        elseif(isset($cryptoOn) && !isset($cryptoData)){$cryptoData = json_decode($response,true);unset($cryptoOn);}

        elseif(isset($wikiOn) && empty($wikiobj)){$_SESSION[$purl.':-:wiki'] = $response;$wikiobj =explode('--]|[--',$response);$wikiTxt = $wikiobj[1];$wikiobj = json_decode($wikiobj[0],true); unset($wikiOn);}
        elseif(isset($newsOn) && empty($NewsObj)){$NewsObj = array(); $NewsObjN = json_decode($response,true);
            foreach ($NewsObjN['articles'] as $article) {
                $NewsObj[] = [
                    'title' => $article['title'],
                    'description' => $article['description'],
                    'img' => $article['urlToImage'],
                    'url' => $article['url'],
                    'date' => $article['publishedAt']
                ];
            }
            usort($NewsObj, function($a, $b) {
                return strtotime($b['date']) - strtotime($a['date']);
            });
            
            unset($newsOn);}
        elseif(isset($ytOn) && empty($YoutubeObj)){$YoutubeObj = array(); $YoutubeObjN = json_decode($response,true); 
            foreach ($YoutubeObjN['items'] as $item) {
                $title = substr($item['snippet']['title'], 0, 47);
                $description = substr($item['snippet']['description'], 0, 47);
    
                $YoutubeObj[] = array(
                    'title' => $title,
                    'description' => $description,
                    'url' => $item['id']['videoId'],
                    'thumb' => str_replace('https://i.ytimg.com/vi/','',$item['snippet']['thumbnails']['medium']['url']),
                    'date' => $item['snippet']['publishTime']
                );
            }        
            unset($ytOn);}

        elseif(isset($shopOn) && !isset($ShopObj)){$ShopObj = json_decode($response,true); unset($shopOn);}
    }    
    curl_multi_remove_handle($multiHandle, $handle);
    }
curl_multi_close($multiHandle);
}
}
else{
    $obj = json_decode(file_get_contents($Googlefile), true);
    $redditObj = json_decode(file_get_contents($RedditFile),true);
    $ddgObj = json_decode(file_get_contents($DdgFile),true);
    $related = json_decode(file_get_contents('./Controller/dev/sug.json'), true);
    $OpenWeatherObj = json_decode(file_get_contents($OpenWeatherFile), true);
    $OpenWeatherForecastObj = json_decode(file_get_contents($OpenWeatherForecastFile), true);
    $ipObj = json_decode(file_get_contents('./Controller/dev/ip.json'), true);
    $NewsObj = json_decode(file_get_contents('./Controller/dev/news.json'), true);
    $YoutubeObj = json_decode(file_get_contents('./Controller/dev/yt.json'),true);
    $WordnikObj = json_decode(file_get_contents('./Controller/dev/def.json'), true);

    $wikiobj =explode('--]|[--',file_get_contents('./Controller/dev/wiki.txt'));$wikiTxt = $wikiobj[1];$wikiobj = json_decode($wikiobj[0],true);
}

    //Engines
    include 'engines/web/google.php';
    include 'engines/web/google2.php';
    include 'engines/web/qwant.php';
    include 'engines/web/brave.php';
    include 'engines/web/mojeek.php';
    include 'engines/web/promo.php';
    include 'engines/web/prieco.php';
    //Addons
    include 'addons/wiki.php';
    include 'addons/hideQuery.php';
    include 'addons/reddit.php';
    include 'addons/related.php';
    
    include 'addons/news.php';
    include 'addons/yt.php';

    include 'addons/pHigh.php';
    include 'addons/shop.php';

    include 'addons/small/ip.php';

    if (preg_match('/\b(?:random|rand)\b/i', $purl)) {include 'addons/small/random.php';}
    if (preg_match('/\b(?:calculator|calc)\b/i', $purl) && !isset($_COOKIE['DisWid'])){include 'addons/small/calculator.php';}
    if (preg_match('/\b(?:color|col)\b/i', $purl) && !isset($_COOKIE['DisWid'])){include 'addons/small/color.html';}
    include 'addons/small/define.php';
    if(isset($OpenWeatherObj)){include 'addons/small/weather.php';}
    include 'addons/small/crypto.php';
    include 'addons/small/convert.php';
    //include 'addons/small/didyoumean.php';
    if(!$dev){include 'addons/summarizer/sum.php';}

    //Next page
    function nextPage($purl,$page){
        if($page > 0){
            echo '<div class="nextPage" style="margin-left: calc(9vw + (clamp(552px, 675px, 90vw) - 210px) / 2);">';
            if(!isset($_COOKIE['hQuery'])){
            echo '<a href="/?q=',htmlspecialchars($purl, ENT_QUOTES | ENT_HTML5, 'UTF-8'),'&page=',$page-1,'">';
            }
            else{
                echo '<form method="POST" action="" style="display:inline;">
                <input type="hidden" name="q" value="', $purl,'">
                <input type="hidden" name="page" value="', $page-1 ,'">
                ';
            }
                echo '<button type="submit">⬅ Back</button>';
                if(!isset($_COOKIE['hQuery'])){echo '</a>';}
                else{echo '</form>';}
            echo '<p style="float:left;color:#8f8f8f;padding:10px;padding-top:22px;">Page: ',$page+1,'</p>';

            if(!isset($_COOKIE['hQuery'])){
                echo '<a href="/?q=',htmlspecialchars($purl, ENT_QUOTES | ENT_HTML5, 'UTF-8'),'&page=',$page+1,'">';
                }
                else{
                    echo '<form method="POST" action="" style="display:inline;">
                    <input type="hidden" name="q" value="', $purl,'">
                    <input type="hidden" name="page" value="', $page+1 ,'">
                    ';
                }
                echo'<button type="submit">Next ⮕</button>';
                if(!isset($_COOKIE['hQuery'])){echo '</a>';}
                else{echo '</form>';}
                echo '</div>
            <style>@media (max-width: 890px) {.nextPage {width:calc(50% + 115px);}}</style>';
        }
        else{
            echo '<div class="nextPage" style="margin-left: calc(9vw + (clamp(552px, 675px, 90vw) - 210px) / 2);">';
            if(!isset($_COOKIE['hQuery'])){
                echo '<a href="/?q=',htmlspecialchars($purl, ENT_QUOTES | ENT_HTML5, 'UTF-8'),'&page=',$page+1,'">';
                }
                else{
                    echo '<form method="POST" action="">
                    <input type="hidden" name="q" value="', $purl,'">
                    <input type="hidden" name="page" value="', $page+1 ,'">
                    ';
                }    
                echo '<button type="submit">Next ⮕</button>';

                if(!isset($_COOKIE['hQuery'])){echo '</a>';}
                else{echo '</form>';}
            echo '</div>
            <style>@media (max-width: 890px) {.nextPage {width:calc(50% + 40px);}}</style>';
        }
    }
    
    $loaded = array_fill(0, 7, false);
    if($page > 0){
        $results = qwant(null, $loaded, $Bpurl, $page);
        foreach($results as &$rs){
            echo $rs;
        }
        nextPage($purl, $page);
        return;
    }

    ##
    #Printing
    ##
    //Addons
    $simImg = $wiki = $news = $reddit = $youtube = $relatedP = '';
    if(!$dev){
    if(isset($_COOKIE['hQuery'])){$hideQueryCopy = hideQuery($Bpurl);}
    else{$hideQueryCopy = null;}
    if(isset($wikiobj)){if(empty($wikiobj['title'])){$wikiobj['title']=$purl;} $wiki = wiki($purl, $wikiobj, $wikiTxt, $ddgObj, $ImpProfiles ?? null, $hideQueryCopy); $simImg = $wiki[1]; $wiki = $wiki[0];}
    if(isset($NewsObj)){$news = search_news($NewsObj);}
    if(isset($YoutubeObj)){$youtube = youtube($YoutubeObj);}
    if(isset($ShopObj)){$shop = shop($ShopObj);}
    }
    $privateAltOut = pHigh($purl);if($privateAltOut != ''){$loaded[6] = true;}
    $reddit = search_reddit($redditObj);
    $relatedP = related($related, $simImg);
    $elseWhere = '<div class="findelsewhere"><p style="float:left;">Search with </p>
    <div style="min-width:340px; float:left;">
    <a href="https://startpage.com/do/metasearch.pl?query='.$purl.'"><button>Startpage</button></a>
    <a href="https://duckduckgo.com/?q='.$purl.'"><button>DuckDuckGo</button></a>
    <a href="https://search.brave.com/search?q='.$purl.'"><button>Brave</button></a>
    <a href="https://www.mojeek.com/search?q='.$purl.'"><button>Mojeek</button></a>
        </div>
    </div>';

    //Check which addons loaded
    if($wiki != ''){$loaded[1] = true;}
    if($shop != ''){$loaded[2] = true;}
    if($news != ''){$loaded[3] = true;}
    if($reddit != ''){$loaded[4] = true;}
    if($youtube != ''){$loaded[5] = true;}
    if($relatedP != ''){$loaded[6] = true;}

    //Backup call and turn off API
    $apiErrors = ['{"error":{"code":','{"response":{"status":"OK"','{"status":"error",'];
    $apiFailed = true;
    
    function disFile($name){
        file_put_contents($name,time());
        return true;
    }
    while($apiFailed){
        $newCall = false;

        if($searchId==0 && strpos(json_encode($obj), $apiErrors[0]) !== false){
            $newCall=disFile('disGoogle.txt');
            $disabled[0]=1;
        }
        elseif($searchId==1 && ($g2obj['status']=='error' || empty($g2obj))){
            $newCall=disFile('disGoogle2.txt');
            $disabled[1]=1;
        }
        elseif($searchId==2 && (strpos(json_encode($QWantObj), $apiErrors[2]) !== false || !isset($QWantObj['data']['result']['items']['mainline']))){
            $newCall=disFile('disBing.txt');
            $disabled[2]=1;
        }
        elseif($searchId==3 && (strpos(json_encode($QWantObj), $apiErrors[2]) !== false || !isset($QWantObj['data']['result']['items']['mainline']))){
            $newCall=disFile('disBing2.txt');
            $disabled[3]=1;
        }
        //Add Brave
        elseif($searchId==5 && strpos(json_encode($MojeekObj), $apiErrors[1]) === false){
            $newCall=disFile('disMojeek.txt');
            $disabled[5]=1;
        }


        if(!$newCall){$apiFailed=false;}

        if($newCall){
        $returnCall = backupCall($searchId, $disabled, $complex, $Bpurl, $MojeekFile, $lang);
        $searchId = $returnCall[1];

        switch($returnCall[1]){
            case 1:$g2obj = $returnCall[0];break;
            case 2:case 3:$QWantObj = $returnCall[0];break;
            case 4:$BraveObj = $returnCall[0];break;
            case 5:$MojeekObj = $returnCall[0];break;
        }
        }
    }

    //Engines
    if($obj != ''){$results = google($obj, $loaded);}
    if(!isset($results[0]) && isset($g2obj)){$results = google2($g2obj, $loaded);}
   if(!isset($results[0]) && isset($QWantObj)){$results = qwant($QWantObj, $loaded, $Bpurl, $page);}
   if(!isset($results[0]) && isset($BraveObj)){$results = brave($BraveObj, $loaded, $Bpurl);}
   if(!isset($results[0]) && isset($MojeekObj)){$results = mojeek($MojeekObj, $loaded);}
    
    if(!isset($_COOKIE['safe']) && !isset($_COOKIE['time'])){
    if($obj != ''){$_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']] = 'obj +=+ '.json_encode($obj);}
    if(isset($g2obj)){$_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']] = 'g2obj +=+ '.json_encode($g2obj);}
    if(isset($QWantObj)){$_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']] = 'qwant +=+ '.json_encode($QWantObj);}
    if(isset($BraveObj)){$_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']] = 'brave +=+ '.$BraveObj;}
    if(isset($MojeekObj)){$_SESSION[$purl.$lang.$loc.$safeS.$_COOKIE['time']] = 'mojeek +=+ '.json_encode($MojeekObj);}

    if(isset($ShopObj)){$_SESSION[$purl.':-:shop'] = json_encode($ShopObj);}
    if(isset($wikiobj)){$_SESSION[$purl.':-:wiki'] = json_encode($wikiobj).'--]|[--'.$wikiTxt.'--]|[--'.json_encode($ddgObj);}
    if(isset($NewsObj)){$_SESSION[$purl.':-:new'] = $NewsObj;}
    if(isset($redditObj)){$_SESSION[$purl.':-:red'] = $redditObj;}
    if(isset($YoutubeObj)){$_SESSION[$purl.':-:yt'] = $YoutubeObj;}
    if(isset($related)){$_SESSION[$purl.':-:rel'] = $related;}
    }
    
    //Print
        //Query URL
    if(preg_match('/^(https?:\/\/)?([a-zA-Z0-9-]+\.)+[a-zA-Z0-9]{2,}(:\d{2,5})?(\/\S*)*$/', preg_replace('/\s+$/u', '', $purl))){
        echo "<div class='output' style='border-radius:20px;border: solid 3px green;margin-bottom:15px;padding: 5px;display:flex;flex-direction: column;flex-wrap: wrap;justify-content: space-around;align-items: center;'>
        <p style='text-align:center;'><b>Looks like you're searching for a website!</b><br>
        If you'd like us to redirect you there, simply click the button below:</p>
        <a style='padding-bottom:5px;' href='";if(strpos($purl, 'https://') === false && strpos($purl, 'http://') === false){echo 'https://';}
            echo $purl,"'";
        if(isset($_COOKIE['new'])){echo 'target="_blank"';}
        echo '><button class="redirectMe">Redirect me!</button></a>
        </div>';
    }
    
    if(isset($results))
    {
    echo $insAnswer, $results[0], $privateAltOut,
    $wiki,
    $results[1],
    $shop,
    $results[2], $results[3],
    $news,
    $results[4], $results[5],
    $reddit,
    $results[6], $results[7],
    $youtube,$results[8],
    $relatedP;
    if(count($results)>8){
        for ($i = 9; $i < count($results); $i++) {
            echo $results[$i];
          }
    }
    echo $elseWhere;
   if(!$dev){
    $priecoTime = microtime(true);
    if(isset($_SESSION[$purl.':-:pri'])){$priecoResults = explode('-<+>-', $_SESSION[$purl.':-:pri']);}
    else{$priecoResults = prieco($purl, $conn, $loc, $lang);}
    $priecoTime = microtime(true) - $priecoTime;

    $pRes = '';
    for($i = 0; $i < 10; $i++){
        echo $priecoResults[$i];
        $pRes .= $priecoResults[$i].'-<+>-';
    }
    $_SESSION[$purl.':-:pri'] = $pRes;
}
    nextPage($purl,$page);
}
else{
    echo '<div style="width: 100vw;
    height: 50vh;
    display: flex;
    align-content: center;
    flex-wrap: wrap;
    justify-content: center;
    align-items: center;
    filter:brightness(0)invert(0.5);"><h1>No results found!</h1><img src="/View/icon/no_link.svg"
    style="width:100px;height:auto;"></div>';
}

if(!isset($_COOKIE['DisWid'])){
    echo "<script>

    function animTypeAdd(sumResOutElement){
        if(!sumResOutElement.classList.contains('typeAnim')){
            sumResOutElement.classList.add('typeAnim');

            setTimeout(() => {
                sumResOutElement.classList.remove('typeAnim');
            }, 1000);
              
        }
    }

  document.addEventListener('DOMContentLoaded', () => {
    const sumResElements = document.querySelectorAll('img#sumRes');
  
    sumResElements.forEach(element => {
      element.addEventListener('click', () => {
        const sumResOutElement = element.nextElementSibling;
        const sumOutElement = sumResOutElement.querySelector('#sumOut');
        const sumImgElement = sumResOutElement.querySelector('#sumImgs');
        const sumIPElement = sumResOutElement.querySelector('#sumIP');
        const sumSSLElement = sumResOutElement.querySelector('#sumSSL');
        const sumSpeedElement = sumResOutElement.querySelector('#sumSpeed');
        
        if (sumOutElement.textContent === '') {
            sumOutElement.textContent = 'Loading...';
          animTypeAdd(sumResOutElement);
        const url = element.getAttribute('data-url');
        const text = fetch('api/summarize.php/?url=' + url)
        .then(responseData => responseData.json())
        .then(data => {
          if (data.summary === '') {
            sumOutElement.textContent = 'Couldn\'t summarize';
          } else {
            const imageUrls = data.images.slice(0, 8);
            imageUrls.forEach(url => {
      const imgElement = document.createElement('img');
      imgElement.src = '/Controller/functions/proxy.php?q='+url;      
      sumImgElement.appendChild(imgElement);
    });
    let sl = false;
    if(data.ssl == '443'){
        sl = true;
    }
            sumOutElement.textContent = data.summary;
            sumIPElement.textContent = 'IP: '+data.ip;
            sumSSLElement.textContent = 'SSL: '+sl;
            sumSpeedElement.textContent = 'Speed: '+data.speed+' sec';
          }
          animTypeAdd(sumResOutElement);
        });
        
      }
  
      const parent = element.parentElement;
      const sibling = parent.querySelector('.snippet');

    if (sibling.style.display == 'none') {
        sibling.style.display = 'block';
        sumResOutElement.style.display = 'none';
    } else {
      
        sumResOutElement.style.display = 'block';
        sibling.style.display = 'none';
        animTypeAdd(sumResOutElement);

  }
       
      });
    });
  });
  
  </script>
    ";
  }
}
if ($type === 'image') {

     //Next page
     function nextPage($purl,$page, $size, $color, $type, $time, $right){
        if($page > 0){
            echo '<div class="nextPage" style="width: 100vw;display: flex;justify-content: center;"><a href="/?image&imgsize=' , $size , '&imgcolor=' , $color , '&imgtype=' , $type , '&imgtime=' , $time , '&imglicence=' , $right , '&q=' , htmlspecialchars($purl, ENT_QUOTES | ENT_HTML5, 'UTF-8'),'&page=',$page-1,'"><button>⬅ Back</button></a>
            <p style="float:left;color:#8f8f8f;padding:10px;padding-top:22px;">Page: ',$page+1,'</p>
            <a href="/?image&imgsize=' , $size , '&imgcolor=' , $color , '&imgtype=' , $type , '&imgtime=' , $time , '&imglicence=' , $right , '&q=' , htmlspecialchars($purl, ENT_QUOTES | ENT_HTML5, 'UTF-8'),'&page=',$page+1,'"><button>Next ⮕</button></a></div>
            <style>@media (max-width: 890px) {.nextPage {width:calc(50% + 115px);}}</style>';
        }
        else{
            echo '
            <div class="nextPage" style="width: 100vw;display: flex;justify-content: center;"><a href="/?image&imgsize=' , $size , '&imgcolor=' , $color , '&imgtype=' , $type , '&imgtime=' , $time , '&imglicence=' , $right , '&q=' , htmlspecialchars($purl, ENT_QUOTES | ENT_HTML5, 'UTF-8'),'&page=',$page+1,'"><button>Next ⮕</button></a></div>
            <style>@media (max-width: 890px) {.nextPage {width:calc(50% + 40px);}}</style>';
        }
    }

    if ($pixabay) {
        echo '<form method="post" action="">
        <input type="submit" style="margin-left:9vw;"class="imgtoolsOption" value="Back" name="imgback" />
        </form>';
        $Pix = curl_init();
        curl_setopt($Pix, CURLOPT_URL, $PixabayFile);
        curl_setopt($Pix, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
        curl_setopt($Pix, CURLOPT_CONNECTTIMEOUT, 2.5);
        curl_setopt($Pix, CURLOPT_RETURNTRANSFER, true);

        $PixObj = json_decode(curl_exec($Pix), true);
        curl_close($Pix);

        echo '<div style="display:flex;margin-top:30px;flex-wrap:wrap;"><br>        
        <div>
         <div tabindex="0" class="imgoutbtn">
             <img src ="/View/img/pix.svg"class="imgout"><p style="color:black;padding-left: 10px;padding-right: 10px;">
         </div>
 
        
         <div class="bigimgout">           
                 <img src ="/View/img/pix.svg">
                 <br>
                 <h3>Thank you Pixabay for providing PriEco with these images</h3><br>
                 <p>From website: https://pixabay.com/</p><br>
                 <div class="bigimgbtn"><a href="https://pixabay.com/"><button class="imgtoolsOption">Go to website</button></a><br>
                 <a href="https://pixabay.com/static/img/logo.svg"> <button class="imgtoolsOption">Go to image</button></a></div>  
                 <div style="display: flex;justify-content: center;"><button class="bigimgclose imgtoolsOption">«Close</button></div>                      
         </div>
       </div>
         ';

        foreach ($PixObj['hits'] as &$item) {
            echo '
           <div class="imgoutdiv">
           <div tabindex="0"  class="imgoutbtn">
               <img style="max-height: 100%;" src="/Controller/functions/proxy.php?q=', $item['webformatURL'], '"class="imgout">
               </div>
               
           <div class="bigimgout">           
           <img src ="/Controller/functions/proxy.php?q=', $item['largeImageURL'], '">
           <br>
           <h3>From user: ', $item['user'], '</h3><br>
           <p>From website: ', $item['largeImageURL'], '</p><br>
           <div class="bigimgbtn"><a href="', $item['pageURL'], '"><button class="imgtoolsOption">Go to website</button></a><br>
           <a href="', $item['largeImageURL'], '"> <button class="imgtoolsOption">Go to image</button></a></div>         
           <div style="display: flex;justify-content: center;"><button class="bigimgclose imgtoolsOption">«Close</button></div>         
           </div>      
         </div>
             ';
        }
        echo '</div>';
        return;
    }

    if(!file_exists('disBing.txt') || !file_exists('disBing2.txt') || isset($_SESSION[$Bpurl.$page.$imgsize.$imgcolor.$imgtype.$imgtime.$imgright.':-:imgBing'])){
    include 'Model/imgset.php';
    include 'Controller/functions/engines/img/bing.php';
    }
    else{
        include 'Controller/functions/engines/img/openverse.php';
    }
}
if ($type == 'video') {
    if (!$dev) {
        $YoutubeObj = array();
        $result = $conn->query("SELECT * FROM `youtube` WHERE `title` LIKE '%$purl%' OR `description` LIKE '%$purl%' LIMIT 200");
        if ($result->num_rows > 0) {
            while ($row = $result->fetch_assoc()) {
                $YoutubeObj[] = $row;
            }
        }
        usort($YoutubeObj, function ($a, $b) {
            $timestampA = strtotime($a['date']);
            $timestampB = strtotime($b['date']);
        
            if ($timestampA === $timestampB) {return 0;}
            return ($timestampA > $timestampB) ? -1 : 1;
        });
        if(count($YoutubeObj)<3){$indexY = false;unset($YoutubeObj);}
        else{$indexY = true;}
    if(!$indexY){
        $YoutubeCurl = curl_init();
        curl_setopt($YoutubeCurl, CURLOPT_URL, $YoutubeFile);
        curl_setopt($YoutubeCurl, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
        curl_setopt($YoutubeCurl, CURLOPT_CONNECTTIMEOUT, 2.5);
        curl_setopt($YoutubeCurl, CURLOPT_RETURNTRANSFER, true);
        $YoutubeObjN = json_decode(curl_exec($YoutubeCurl), true);
        curl_close($YoutubeCurl);
       
        foreach ($YoutubeObjN['items'] as $item) {
            $title = substr($item['snippet']['title'], 0, 47);
            $description = substr($item['snippet']['description'], 0, 47);

            $YoutubeObj[] = array(
                'title' => $title,
                'description' => $description,
                'url' => $item['id']['videoId'],
                'thumb' => str_replace('https://i.ytimg.com/vi/','',$item['snippet']['thumbnails']['medium']['url']),
                'date' => $item['snippet']['publishTime']
            );
        }        
    }
    } else {
        $YoutubeObj =  json_decode(file_get_contents($YoutubeFile), true);
    }
    echo '<div style="display:flex;margin-top:30px;flex-wrap:wrap;justify-content: center;"><br>';

    foreach ($YoutubeObj as &$item) {   

        echo '

                    <div class="imgoutdiv">
                    <a style="text-decoration: none;" href="https://www.youtube.com/watch?v=' . $item['url'] . '"';
                    if (isset($_COOKIE['new'])) {
                        echo 'target="_blank"';
                    }
                    echo'>
                    <button class="imgoutbtn" style="background-color: var(--result-bg);;border-radius: 20px;height:auto;">
            <img src ="/Controller/functions/proxy.php?q=https://i.ytimg.com/vi/',  $item['thumb'], '"class="imgout" loading="lazy">
            <div class="imgoutlink">
            <p style="font-size:12px;font-weight:bold;">', $item['title'],'</p>
            </div>
            </button>
        </a>
        </div>
              ';
    }
    echo '</div>';
}

if ($type == 'news') {
    if (!$dev) {
        $NewsObj = array();
     $result = $conn->query("SELECT * FROM `news` WHERE `title` LIKE '%$purl%' OR `description` LIKE '%$purl%'");
     if ($result->num_rows > 0) {
         while ($row = $result->fetch_assoc()) {
             $NewsObj[] = $row;
         }
     }
     usort($NewsObj, function ($a, $b) {
         $timestampA = strtotime($a['date']);
         $timestampB = strtotime($b['date']);
     
         if ($timestampA === $timestampB) {return 0;}
         return ($timestampA > $timestampB) ? -1 : 1;
     });
     if(count($NewsObj)<3){$indexN = false;}
     else{$indexN = true;}
     if(!$indexN){
        $newsCh = curl_init($NewsFile);
        curl_setopt($newsCh, CURLOPT_RETURNTRANSFER, true);
        curl_setopt($newsCh, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');
        curl_setopt($newsCh, CURLOPT_CONNECTTIMEOUT, 2);
        curl_setopt($newsCh, CURLOPT_TIMEOUT, 2.5);
        $NewsObj = array(); $NewsObjN = json_decode(curl_exec($newsCh),true);
            foreach ($NewsObjN['articles'] as $article) {
                $NewsObj[] = [
                    'title' => $article['title'],
                    'description' => $article['description'],
                    'img' => $article['urlToImage'],
                    'url' => $article['url'],
                    'date' => $article['publishedAt']
                ];
            }
            usort($NewsObj, function($a, $b) {
                return strtotime($b['date']) - strtotime($a['date']);
            });
        curl_close($newsCh);
    }
    } else {
        $NewsObj =  json_decode(file_get_contents($NewsFile), true);
    }

    $i = 0;
    foreach ($NewsObj as &$news) {
        $title = str_replace('<', '', $news['title']);
        $title = str_replace('>', '', $title);
        $desc = str_replace('<', '', $news['description']);
        $desc = str_replace('>', '', $desc);
        echo '<div class="output"';
        if ($i==0) {
            echo 'style="border-top-left-radius: 20px;
            border-top-right-radius: 20px;padding-top: 18px;"';
        }
        elseif($i == count($NewsObj)-1){
            echo 'style="border-bottom-left-radius: 20px;
            border-bottom-right-radius: 20px;"';
        }
        echo ' id="output">';
        if ($news['img'] != '' and !isset($_COOKIE['datasave'])) {
            echo '<img loading="lazy" src="/Controller/functions/proxy.php?q=', $news['img'], '" class="OutSideImg">';
        }
        echo '<a ';
        if (isset($_COOKIE['new'])) {
            echo 'target="_blank"';
        }
        echo 'href="', $news['url'], '">';
        if (strpos($news['url'], 'https://') !== false && !isset($_COOKIE['datasave'])) {
            echo '<img class="Outfavicon" style="margin-top:unset;" loading="lazy" src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'. parse_url($news['url'])['host']. '">';
        }
    echo '<p class="OutTitle" style="font-size:16px;padding-top:0;padding-bottom: 5px;">', $title, '</p></a>
    <p class="snippet"style="font-size:12px;padding-bottom: 18px;">', $desc, '</p>
    ';
        if ($providers == 'on') {
            echo '<p class="resProvider">Newsapi</p>';
        }
        echo '</div>';

        ++$i;
    }
}
elseif($type=='shop'){
$tmp = ($loc == 'all') ? 'us' : $loc;

$shopCh = curl_init('https://api.yadore.com/v2/offer?market='.$tmp.'&keyword='.urlencode($purl).'&precision=fuzzy&limit=100');
    curl_setopt($shopCh, CURLOPT_RETURNTRANSFER, true);

    $headers = ['API-Key: '. $_ENV['SHOP_API_KEY'],];
    curl_setopt($shopCh, CURLOPT_HTTPHEADER, $headers);

$ShopObj = json_decode(curl_exec($shopCh),true);

include 'Model/shopSet.php';
    echo '<div style="display:flex;margin-top:40px;flex-wrap:wrap;justify-content:center;width:100vw;"><br>';
              foreach ($ShopObj['offers'] as &$item) { 
                if(isset($_GET['shopMin']) && $item['price']['amount'] < $_GET['shopMin']){continue;}
                if(isset($_GET['shopMax']) && $item['price']['amount'] > $_GET['shopMax']){continue;}
                         
                  echo '
                 <div class="imgoutdiv">
                  <div tabindex="0" class="imgoutbtn" style="background-color: var(--result-bg);border-radius: 20px;height:auto;">
                  <a style="color: unset; cursor:pointer;text-decoration:none;" href="',$item['clickUrl'],'"';if (isset($_COOKIE['new'])) {echo 'target="_blank';}echo'>
                      <img src="Controller/functions/proxy.php?q=',urlencode($item['thumbnail']['url']),'" class="imgout" style="width:230px;height:230px;background-color: #00000007;">
                      <p class="shopTitle">',$item['title'],'</p>
                      <p class="shopPrice">$',$item['price']['amount'],'</p><br>
                      <p class="shopLogo shopPrice">';
                      if($item['merchant']['logo']['exists']){
                      echo '<img src="/Controller/functions/proxy.php?q=',$item['merchant']['logo']['url'],'">';
                      }
                      echo $item['merchant']['name'],'</p>
                      </a>
                  </div>
                  </div>
                  ';
              }
              echo '</div>';

}
