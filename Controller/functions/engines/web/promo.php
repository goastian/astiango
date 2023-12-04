<?php
function promo($promoobj, $purl)
{
    $promoRes = '';
    $i = 0;
    $j=0;
    foreach ($promoobj['promotions'] as $promo) {
        foreach ($promo['keyWord'] as $key) {
            if($j == 1){break;}
            $keys = strtolower($purl);
            if (strpos($keys, $key) !== false) {
                $promoRes .= '<div class="';
                $promoRes .= ' output" id="output"><p style="color:grey;font-style: italic;font-size: 16px;">Ad:</p><h4 style="padding-top:0"><a style="color: #3391ff" ';
                if ($_COOKIE['new'] === 'on') {
                    $promoRes .= 'target="_blank"';
                }
                $promoRes .= 'href="' . $promo['link'] . '">' . $promo['htmlTitle'] . '</a></h4>';
                $promoRes .= '<p class="resLink">' . $promo['displayLink'] . '</p>';
                $promoRes .= ' <p class="snippet">' . $promo['snippet'] . '</p></div>';
                ++$j;
            }
        }
    }
    echo $promoRes;
}