<?php
$json = file_get_contents('Controller/value/convert.json');
$conversionTable = json_decode($json, true);

$unC = 0;
$posUnits = explode(' ', $purl);

$j = count($conversionTable);
for ($i = 0; $i < $j; $i++) {
    foreach ($posUnits as &$pU) {
        if (isset($conversionTable["$i"][$pU])) {
            if (!isset($fromUnit)) {
                $fromUnit = $pU;
                ++$unC;
            } elseif (!isset($toUnit)) {
                $toUnit = $pU;
                ++$unC;
            }
        }
        if ($unC >= 2) {
            break;
        }
    }
    if ($unC >= 2) {
        break;
    }
    unset($fromUnit);
    unset($toUnit);
}

if ($unC >= 2) {
    preg_match_all('/\d+(\.\d+)?/', $purl, $matches);
    if (!empty($matches[0])) {
        $number = $matches[0][0];
    }

    if (isset($fromUnit) && isset($toUnit) && isset($number)) { 
        $result = floatval($number) / floatval($conversionTable["$i"][$toUnit]) * floatval($conversionTable["$i"][$fromUnit]); 
        echo '
        <div class="output" style="margin-bottom: 15px;border-radius: 20px;padding: 10px;">
            <div class="display">
                <input id="screen" style="margin-bottom:unset;" type="text" value="', $result, ' ', $toUnit, '">
            </div>
        </div>';
    }
}
?>
