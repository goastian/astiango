<?php
if (isset($cryptoData['image']['large'])) {
    $c = 'max';
    if(isset($_GET['c'])){$c=$_GET['c'];}
    
    $logo = $cryptoData['image']['large'];
    $currentPrice = $cryptoData['market_data']['current_price']['usd'];

function getCoinPriceHistory($coinName, $c) {
    $apiEndpoint = 'https://api.coingecko.com/api/v3';
    $coinName = urlencode($coinName);
    $url = $apiEndpoint . "/coins/$coinName/market_chart?vs_currency=usd&days=".$c;
    $data = file_get_contents($url);
    return json_decode($data, true);
}

$coinPriceHistory = getCoinPriceHistory($purl, $c)['prices'];
$time = array_column($coinPriceHistory, 0);
$price = array_column($coinPriceHistory, 1);
echo '<div class="output"style="margin-bottom: 15px;border-radius: 20px;padding: 10px;">
<div style="display:flex;"><img style="width: 24px;height: 24px;margin-right: 9px;border-radius: 20px;"src="/Controller/functions/proxy.php?q=',$logo,'"><p>',$purl,'</p></div>
<p><b>$',$currentPrice,'</b></p><br>
<div style="display:flex;flex-wrap: wrap;">
';if($c != '1'){echo '<a href="?c=1&q=',$purl,'" style="padding-bottom:5px;">';}echo'<button class="quickSet" style="height:23px;padding:0;margin-left:5px;';if($c != '1'){echo 'border:none;background-color:unset;"';}echo '">Day</button>';if($c != '1'){echo '</a>';}
if($c != '7'){echo '<a href="?c=7&q=',$purl,'" style="padding-bottom:5px;">';}echo'<button class="quickSet" style="height:23px;padding:0;margin-left:5px;';if($c != '7'){echo 'border:none;background-color:unset;"';}echo '">Week</button>';if($c != '7'){echo '</a>';}
if($c != '30'){echo '<a href="?c=30&q=',$purl,'" style="padding-bottom:5px;">';}echo'<button class="quickSet" style="height:23px;padding:0;margin-left:5px;';if($c != '30'){echo 'border:none;background-color:unset;"';}echo '">Month</button>';if($c != '30'){echo '</a>';}
if($c != '365'){echo '<a href="?c=365&q=',$purl,'" style="padding-bottom:5px;">';}echo'<button class="quickSet" style="height:23px;padding:0;margin-left:5px;';if($c != '365'){echo 'border:none;background-color:unset;"';}echo '">Year</button>';if($c != '365'){echo '</a>';}
if($c != 'max'){echo '<a href="?c=max&q=',$purl,'" style="padding-bottom:5px;">';}echo'<button class="quickSet" style="height:23px;padding:0;margin-left:5px;';if($c != 'max'){echo 'border:none;background-color:unset;"';}echo '">All</button>';if($c != 'max'){echo '</a>';}
echo '</div>';

$chartPadding = 50;
$pointInterval = 10;

$dataMin = min($price);
$dataMax = max($price);

$priceF = $price[0];
$priceL =$price[count($price)-1];

$chartLeft = $chartPadding;
$chartRight = 650;
$chartTop = $chartPadding;
$chartBottom = 200 - $chartPadding;

$dataPoints = [];
$pointCount = count($price);
for ($i = 0; $i < $pointCount; $i++) {
    $x = $chartLeft + ($i / ($pointCount - 1)) * ($chartRight - $chartLeft);
    if ($dataMax != $dataMin) {
        $y = $chartBottom - (($price[$i] - $dataMin) / ($dataMax - $dataMin)) * ($chartBottom - $chartTop);
    } else {
        $y = $chartBottom - ($price[$i] - $dataMin) * ($chartBottom - $chartTop);
    }
    $dataPoints[] = [$x, $y];
}

$svgWidth = $chartRight;
$svgHeight = 200;

$txtPadding = 7;
echo '<div class="weatherChart" style="height:200px;visibility:unset;"><svg width="',$svgWidth,'" height="' , $svgHeight , '">

<defs>
  <filter id="shadow" x="0%" y="0%" width="140%" height="140%">
  <feDropShadow dx="0" dy="0" stdDeviation="20" flood-color="';if($priceF<=$priceL){echo'lightgreen';}else{echo 'red';}echo'"/>
  </filter>
</defs>

<polyline points="';
foreach ($dataPoints as $point) {
    echo $point[0] , ',' , $point[1] , ' ';
}
echo '" fill="none" stroke-width="7" stroke="';if($priceF<=$priceL){echo'lightgreen';}else{echo 'red';}echo'" filter="url(#shadow)" />';


foreach ($dataPoints as $point) {
    echo '<circle cx="' , $point[0] , '" cy="' . $point[1] , '" r="1" fill="';if($priceF<=$priceL){echo'lightgreen';}else{echo 'red';}echo'" />';
}

$textOffsetY = -15;
$i = -1;
foreach ($dataPoints as $index => $point) {
    ++$i;
    if (($i % ($pointCount / $txtPadding)) !== 0) {continue;}
    $textX = $point[0];
    $textY = $point[1] + $textOffsetY;
    $text = '$' . round($price[$index], 2);
    echo '<text x="' , $textX , '" y="' , $textY , '" text-anchor="middle" fill="gray">' , $text , '</text>';
}

$i = -1;
$textOffsetY = 25;
foreach ($dataPoints as $index => $point) {
    ++$i;
    if (($i % ($pointCount / $txtPadding)) !== 0) {continue;}

    $textX = $point[0];
    $textY = 180;
    $text = $date = date('Y-m-d', $time[$index] / 1000);
    echo '<text style="font-size:12px;" x="' , $textX , '" y="' , $textY , '" text-anchor="middle" fill="gray">' , $text , '</text>';
}

echo '</svg></div>

<div style="display: flex;flex-wrap: wrap;justify-content: space-between;">
<p style="color: gray;font-size: 12px;">Hight: $',round($dataMax,2),'<br>
Low: $',round($dataMin,2),'</p>
<a href="https://www.coingecko.com/en/coins/',$purl,'"><p style="color: gray;font-size: 12px;">Data from CoinGecko</p></a></div>';

echo'</div>';
}
?>
