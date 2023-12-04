<?php
function google($obj, $loaded)
{
    $google[]=null;
    $i = 0;
    foreach ($obj['items'] as &$item) {
    $google[$i] = '<div class="';

switch ($i){
    case 0:
        if(!$loaded[0] && $loaded[1]) {$google[$i] .= ' mBorderBoth2 mBorderTop ';}
        elseif(!$loaded[0]){$google[$i] .= ' mBorderTop ';}
        elseif($loaded[1]){$google[$i] .= ' mBorderBottom2 ';}
        break;
    case 1:
        if($loaded[1]) {$google[$i] .= ' mBorderTop2 ';}
        if($loaded[2]) {$google[$i] .= ' mBorderBottom ';}
        break;
    case 2:
        if($loaded[2]) {$google[$i] .= ' mBorderTop ';}
       break;
    case 3:
        if($loaded[3]) {$google[$i] .= ' mBorderBottom ';}
        break;
    case 4:
        if($loaded[3]) {$google[$i] .= ' mBorderTop ';}
        break;
    case 5:
        if($loaded[4]) {$google[$i] .= ' mBorderBottom ';}
        break;
    case 6:
        if($loaded[4]) {$google[$i] .= ' mBorderTop ';}
        break;
    case 7:
        if($loaded[5]) {$google[$i] .= ' mBorderBottom ';}
        break;
    case 8:
        if($loaded[5]) {$google[$i] .= ' mBorderTop ';}
        if($loaded[6]) {$google[$i] .= ' mBorderBottom ';}
        break;
    case 9:
        if($loaded[6]) {$google[$i] .= ' mBorderTop ';}
        $google[$i] .= ' mBorderBottom ';
        break;
}

$gurl = str_replace('/',' > ',str_replace('https://','',str_replace('http://','',str_replace('www.','', $item['link']))));
if ( substr_compare($gurl, ' > ', -3) === 0 ) {
$gurl = substr($gurl, 0, -3);
}

$google[$i] .= ' output" id="output">';
if (isset($item['pagemap']['cse_thumbnail'][0]['src']) && !isset($_COOKIE['datasave'])) {
$google[$i] .= '<img loading="lazy" alt="‎" src="/Controller/functions/proxy.php?q='. $item['pagemap']['cse_thumbnail'][0]['src']. '" class="OutSideImg">';
}
if (strpos($item['link'], 'https://') !== false && !isset($_COOKIE['datasave'])) {
$google[$i] .= '<img class="Outfavicon" alt="‎" loading="lazy" src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'. get_string_betweens($item['link'], 'https://', '/'). '">';
}
$google[$i] .= '<a ';
if (isset($_COOKIE['new'])) {
$google[$i] .= 'target="_blank"';
}
$google[$i] .= 'href="'. $item['link']. '" data-sxpr-link>';
$google[$i] .= '<p class="OutTitle">'.strip_tags($item['title']). '</p></a>
<p class="resLink">'. $gurl. '</p>';

if(isset($item['snippet'])){$google[$i] .= '<p class="snippet">'.strip_tags($item['snippet']). '</p>';}
if (isset($_COOKIE['providers'])) {
$google[$i] .= '<p class="resProvider">Google</p>';
}
if(!isset($_COOKIE['DisWid'])){
$google[$i] .= '
<img class="filterImage sumOpen resProvider" id="sumRes" data-url="'.$item['link'].'" src="View/icon/circle-info.svg">
<div id="sumResOut" class="sumOut snippet">
    <div id="sumImgs"></div>
    <p id="sumOut"></p>
    <p id="sumIP"></p>
    <p id="sumSSL"></p>
    <p id="sumSpeed"></p>
</div>';
}
$google[$i].='</div>';
++$i;
    }
return $google;
}