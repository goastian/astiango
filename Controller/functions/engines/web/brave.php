<?php
function getBetween($string, $start = "", $end = ""){
    if (strpos($string, $start)) { // required if $start not exist in $string
        $startCharCount = strpos($string, $start) + strlen($start);
        $firstSubStr = substr($string, $startCharCount, strlen($string));
        $endCharCount = strpos($firstSubStr, $end);
        if ($endCharCount == 0) {
            $endCharCount = strlen($firstSubStr);
        }
        return substr($firstSubStr, 0, $endCharCount);
    } else {
        return '';
    }
}

function brave($BraveObj, $loaded, $Bpurl){
$snippet = str_get_html($BraveObj)->find('div.snippet');

if(isset($snippet)){
$i=0;
$CountBraveSnippets = count($snippet);
if($CountBraveSnippets >= 20){
    $j=0;
    foreach ($snippet as $snip) {
        $href = $snip->find('a', 0);
        if($href == null){
            break;
        }
        ++$j;
    }
    $CountBraveSnippets = $j-1;
}
else{
    $CountBraveSnippets -= 1;
}

$i = 0;
$brave[] = null;
foreach ($snippet as $snip) {
    $href = $snip->find('a', 0);
    if($href == null){continue;}

    if($href->find('div.favicon-wrapper', 0)->outertext != null){
    $href->find('div.favicon-wrapper', 0)->outertext = '';
    }
    $url = $href->href;
    if(!filter_var($url, FILTER_VALIDATE_URL)){
        continue;
    }

    $description = strip_tags($snip->find('div.snippet-description', 0));
    $description = strlen($description)>150 ? substr($description,0,150).'...' : $description;

    $href= strip_tags($href);
    if(strpos($href,' › ') !== false){
        $href=explode(' › ',$href)[0];
        $href = str_replace(str_replace('www.','',parse_url($url)['host']),'',$href);
    }
    $brave[$i] = '<div class="';
    switch ($i){
        case 0:
            if(!$loaded[0] && $loaded[1]) {$brave[$i] .= ' mBorderBoth2 mBorderTop ';}
            elseif(!$loaded[0]){$brave[$i] .= ' mBorderTop ';}
            elseif($loaded[1]){$brave[$i] .= ' mBorderBottom2 ';}
            break;
        case 1:
            if($loaded[1]) {$brave[$i] .= ' mBorderTop2 ';}
            if($loaded[2]) {$brave[$i] .= ' mBorderBottom ';}
            break;
        case 2:
            if($loaded[2]) {$brave[$i] .= ' mBorderTop ';}
           break;
        case 3:
            if($loaded[3]) {$brave[$i] .= ' mBorderBottom ';}
            break;
        case 4:
            if($loaded[3]) {$brave[$i] .= ' mBorderTop ';}
            break;
        case 5:
            if($loaded[4]) {$brave[$i] .= ' mBorderBottom ';}
            break;
        case 6:
            if($loaded[4]) {$brave[$i] .= ' mBorderTop ';}
            break;
        case 7:
            if($loaded[5]) {$brave[$i] .= ' mBorderBottom ';}
            break;
        case 8:
            if($loaded[5]) {$brave[$i] .= ' mBorderTop ';}
            if($loaded[6]) {$brave[$i] .= ' mBorderBottom ';}
            break;
        case 9:
            if($loaded[6]) {$brave[$i] .= ' mBorderTop ';}
            break;
    }
    if($CountBraveSnippets == $i){$brave[$i] .= ' mBorderBottom ';}

        $brave[$i] .= ' output" id="output">';

    if(strpos($url, 'https://') !== false){$brave[$i] .= '<img class="Outfavicon" alt="‎" loading="lazy" src="https://judicial-peach-octopus.b-cdn.net/'.get_string_betweens($url, 'https://', '/').'">';}
        $brave[$i] .= '<a ';
        if (isset($_COOKIE['new'])) {
            $brave[$i] .= 'target="_blank"';
        }
        $gurl = str_replace('/',' > ',str_replace('https://','',str_replace('http://','',str_replace('www.','', $url))));
            if ( substr_compare($gurl, ' > ', -3) === 0 ) {
            $gurl = substr($gurl, 0, -3);
            }
        $brave[$i] .= 'href="' . $url . '" data-sxpr-link>';
        $brave[$i] .= '<p class="OutTitle">'.$href.'</p></a>
    <p class="resLink">' . strip_tags($gurl) . '</p>
    <p class="snippet">' . strip_tags($description) . '</p>';
    if (isset($_COOKIE['providers'])) {
        $brave[$i] .= '<p class="resProvider">Brave</p>';
    }
    if(!isset($_COOKIE['DisWid'])){
    $brave[$i] .= '
    <img class="filterImage sumOpen resProvider" id="sumRes" data-url="'.$url.'" src="View/icon/circle-info.svg">
    <div id="sumResOut" class="sumOut snippet">
    <div id="sumImgs"></div>
    <p id="sumOut"></p>
    <p id="sumIP"></p>
    <p id="sumSSL"></p>
    <p id="sumSpeed"></p>
</div>';
    }
$brave[$i].='</div>';
    ++$i;
}
}
return $brave;
}