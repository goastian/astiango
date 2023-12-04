<?php
function qwant($QWantObj, $loaded, $Bpurl, $page){
        $qwant[]=null;
        $i = 0;
foreach ($QWantObj['data']['result']['items']['mainline'] as &$items) {
    if ($items['type'] === 'web') {
        foreach ($items['items'] as &$item) {
            $qwant[$i] .= '<div class="';

            switch ($i){
                case 0:
                    if(!$loaded[0] && $loaded[1]) {$qwant[$i] .= ' mBorderBoth2 mBorderTop ';}
                    elseif(!$loaded[0]){$qwant[$i] .= ' mBorderTop ';}
                    elseif($loaded[1]){$qwant[$i] .= ' mBorderBottom2 ';}
                    break;
                case 1:
                    if($loaded[1]) {$qwant[$i] .= ' mBorderTop2 ';}
                    if($loaded[2]) {$qwant[$i] .= ' mBorderBottom ';}
                    break;
                case 2:
                    if($loaded[2]) {$qwant[$i] .= ' mBorderTop ';}
                   break;
                case 3:
                    if($loaded[3]) {$qwant[$i] .= ' mBorderBottom ';}
                    break;
                case 4:
                    if($loaded[3]) {$qwant[$i] .= ' mBorderTop ';}
                    break;
                case 5:
                    if($loaded[4]) {$qwant[$i] .= ' mBorderBottom ';}
                    break;
                case 6:
                    if($loaded[4]) {$qwant[$i] .= ' mBorderTop ';}
                    break;
                case 7:
                    if($loaded[5]) {$qwant[$i] .= ' mBorderBottom ';}
                    break;
                case 8:
                    if($loaded[5]) {$qwant[$i] .= ' mBorderTop ';}
                    if($loaded[6]) {$qwant[$i] .= ' mBorderBottom ';}
                    break;
                case 9:
                    if($loaded[6]) {$qwant[$i] .= ' mBorderTop ';}
                    $qwant[$i] .= ' mBorderBottom ';
                    break;
            }
$qwant[$i] .= ' output" id="output">';
            $qwant[$i] .= '<a ';
            if (isset($_COOKIE['new'])) {
                $qwant[$i] .= 'target="_blank"';
            }
            $qwant[$i] .= 'href="'. $item['url']. '" data-sxpr-link>';
            if (strpos($item['url'], 'https://') !== false && !isset($_COOKIE['datasave'])) {
                $qwant[$i] .= '<img loading="lazy" alt="‎" class="Outfavicon" src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'.get_string_betweens($item['url'], 'https://', '/'). '">';
            }
            $gurl = str_replace('/',' > ',str_replace('https://','',str_replace('http://','',str_replace('www.','', $item['url']))));
            if ( substr_compare($gurl, ' > ', -3) === 0 ) {
            $gurl = substr($gurl, 0, -3);
            }

            $description = strip_tags($item['desc']);
            $description = strlen($description)>150 ? substr($description,0,150).'...' : $description;

            $qwant[$i] .= '<p class="OutTitle">'. $item['title']. '</p></a>
                    <p class="resLink">'. $gurl .'</p>
                    <p class="snippet">'.$description . '</p>';
                    if (isset($_COOKIE['providers'])) {
                        $qwant[$i] .= '<p class="resProvider">Bing</p>';
                    }
                    if(!isset($_COOKIE['DisWid'])){
                    $qwant[$i] .='
                    <img class="filterImage sumOpen resProvider" id="sumRes" data-url="'.$item['url'].'" src="View/icon/circle-info.svg">
                    <div id="sumResOut" class="sumOut snippet">
                    <div id="sumImgs"></div>
                    <p id="sumOut"></p>
                    <p id="sumIP"></p>
                    <p id="sumSSL"></p>
                    <p id="sumSpeed"></p>
                </div>';
                    }
                $qwant[$i].= '</div>';
            ++$i;
        }
    }
}
return $qwant;
}