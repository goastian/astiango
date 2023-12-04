<?php
function mojeek($mojeekObj, $loaded){
$mojeek[]=null;

    $i = 0;
    $bottomBorder = count($mojeekObj['response']['results'])-1;

    foreach ($mojeekObj['response']['results'] as &$item) {
        if(!isset($item['url'])){--$bottomBorder;continue;}
    $mojeek[$i] = '<div class="';

    switch ($i){
        case 0:
            if(!$loaded[0] && $loaded[1]) {$mojeek[$i] .= ' mBorderBoth2 mBorderTop ';}
            elseif(!$loaded[0]){$mojeek[$i] .= ' mBorderTop ';}
            elseif($loaded[1]){$mojeek[$i] .= ' mBorderBottom2 ';}
            break;
        case 1:
            if($loaded[1]) {$mojeek[$i] .= ' mBorderTop2 ';}
            if($loaded[2]) {$mojeek[$i] .= ' mBorderBottom ';}
            break;
        case 2:
            if($loaded[2]) {$mojeek[$i] .= ' mBorderTop ';}
           break;
        case 3:
            if($loaded[3]) {$mojeek[$i] .= ' mBorderBottom ';}
            break;
        case 4:
            if($loaded[3]) {$mojeek[$i] .= ' mBorderTop ';}
            break;
        case 5:
            if($loaded[4]) {$mojeek[$i] .= ' mBorderBottom ';}
            break;
        case 6:
            if($loaded[4]) {$mojeek[$i] .= ' mBorderTop ';}
            break;
        case 7:
            if($loaded[5]) {$mojeek[$i] .= ' mBorderBottom ';}
            break;
        case 8:
            if($loaded[5]) {$mojeek[$i] .= ' mBorderTop ';}
            if($loaded[6]) {$mojeek[$i] .= ' mBorderBottom ';}
            break;
        case 9:
            if($loaded[6]) {$mojeek[$i] .= ' mBorderTop ';}
            break;
    }
    if($bottomBorder == $i){$mojeek[$i] .= ' mBorderBottom ';}

    
$gurl = str_replace('/',' > ',str_replace('https://','',str_replace('http://','',str_replace('www.','', $item['url']))));
if ( substr_compare($gurl, ' > ', -3) === 0 ) {
$gurl = substr($gurl, 0, -3);
}

$mojeek[$i] .= ' output" id="output">';
if (strpos($item['url'], 'https://') !== false && !isset($_COOKIE['datasave'])) {
$mojeek[$i] .= '<img class="Outfavicon" alt="‎" loading="lazy" src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'. get_string_betweens($item['url'], 'https://', '/'). '">';
}
$mojeek[$i] .= '<a ';
if (isset($_COOKIE['new'])) {
$mojeek[$i] .= 'target="_blank"';
}
$mojeek[$i] .= 'href="'. $item['url']. '" data-sxpr-link>';
$mojeek[$i] .= '<p class="OutTitle">'. $item['title']. '</p></a>
<p class="resLink">'. $gurl. '</p>';

if(isset($item['desc'])){$mojeek[$i] .= '<p class="snippet">'. $item['desc']. '</p>';}
if (isset($_COOKIE['providers'])) {
$mojeek[$i] .= '<p class="resProvider">Mojeek</p>';
}
if(!isset($_COOKIE['DisWid'])){
$mojeek[$i] .= '
<img class="filterImage sumOpen resProvider" id="sumRes" data-url="'.$item['url'].'" src="View/icon/circle-info.svg">
<div id="sumResOut" class="sumOut snippet">
    <div id="sumImgs"></div>
    <p id="sumOut"></p>
    <p id="sumIP"></p>
    <p id="sumSSL"></p>
    <p id="sumSpeed"></p>
</div>';
}
$mojeek[$i].='</div>';
++$i;
    }
return $mojeek;
}