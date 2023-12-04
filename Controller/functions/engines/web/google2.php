<?php
function google2($g2obj, $loaded){
$google2[]=null;

    $i = 0;
    $bottomBorder = count($g2obj)-1;
    foreach ($g2obj as &$item) {
        if(!isset($item['url'])){--$bottomBorder;continue;}
    $google2[$i] = '<div class="';

    switch ($i){
        case 0:
            if(!$loaded[0] && $loaded[1]) {$google2[$i] .= ' mBorderBoth2 mBorderTop ';}
            elseif(!$loaded[0]){$google2[$i] .= ' mBorderTop ';}
            elseif($loaded[1]){$google2[$i] .= ' mBorderBottom2 ';}
            break;
        case 1:
            if($loaded[1]) {$google2[$i] .= ' mBorderTop2 ';}
            if($loaded[2]) {$google2[$i] .= ' mBorderBottom ';}
            break;
        case 2:
            if($loaded[2]) {$google2[$i] .= ' mBorderTop ';}
           break;
        case 3:
            if($loaded[3]) {$google2[$i] .= ' mBorderBottom ';}
            break;
        case 4:
            if($loaded[3]) {$google2[$i] .= ' mBorderTop ';}
            break;
        case 5:
            if($loaded[4]) {$google2[$i] .= ' mBorderBottom ';}
            break;
        case 6:
            if($loaded[4]) {$google2[$i] .= ' mBorderTop ';}
            break;
        case 7:
            if($loaded[5]) {$google2[$i] .= ' mBorderBottom ';}
            break;
        case 8:
            if($loaded[5]) {$google2[$i] .= ' mBorderTop ';}
            if($loaded[6]) {$google2[$i] .= ' mBorderBottom ';}
            break;
        case 9:
            if($loaded[6]) {$google2[$i] .= ' mBorderTop ';}
            break;
    }
    if($bottomBorder == $i){$google2[$i] .= ' mBorderBottom ';}
    
    
$gurl = str_replace('/',' > ',str_replace('https://','',str_replace('http://','',str_replace('www.','', $item['url']))));
if ( substr_compare($gurl, ' > ', -3) === 0 ) {
$gurl = substr($gurl, 0, -3);
}

$google2[$i] .= ' output" id="output">';
if (strpos($item['url'], 'https://') !== false && !isset($_COOKIE['datasave'])) {
$google2[$i] .= '<img class="Outfavicon" alt="‎" loading="lazy" src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'. get_string_betweens($item['url'], 'https://', '/'). '">';
}
$google2[$i] .= '<a ';
if (isset($_COOKIE['new'])) {
$google2[$i] .= 'target="_blank"';
}
$google2[$i] .= 'href="'. $item['url']. '" data-sxpr-link>';
$google2[$i] .= '<p class="OutTitle">'. $item['title']. '</p></a>
<p class="resLink">'. $gurl. '</p>';

if(isset($item['description'])){$google2[$i] .= '<p class="snippet">'. $item['description']. '</p>';}
if (isset($_COOKIE['providers'])) {
$google2[$i] .= '<p class="resProvider">Google</p>';
}
if(!isset($_COOKIE['DisWid'])){
$google2[$i] .= '
<img class="filterImage sumOpen resProvider" id="sumRes" data-url="'.$item['url'].'" src="View/icon/circle-info.svg">
<div id="sumResOut" class="sumOut snippet">
    <div id="sumImgs"></div>
    <p id="sumOut"></p>
    <p id="sumIP"></p>
    <p id="sumSSL"></p>
    <p id="sumSpeed"></p>
</div>';
}
$google2[$i].='</div>';
++$i;
    }
return $google2;
}