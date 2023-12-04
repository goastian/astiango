<?php

function related($rsArray, $simImg){
      $rPrint = false;
$i=0;
$related = '<style>
.relSea{
--rel-img: url("'.$simImg.'");
} </style>
<p class="sectionTitle">🔗 Related searches</p>
<div class="relSea output" style="border-radius: 20px;margin-bottom:15px;" id="output">';

foreach ($rsArray as &$item) { 
    if($i==0){
        ++$i;
        continue;
    } 
                $rPrint = true;
                if(!isset($_COOKIE['hQuery'])){
                $related .= '<a href="?q='.urlencode($item['name']).'">';
                }
                else{
                    $related .= '<form method="POST" action="">
                    <input type="hidden" name="q" value="'. $item['name'] .'">';
                }
                
                $related .= '<button class="socialBtn" style="  color: var(--linkColor);padding: 10px;float: left;margin-top: 10px;display: flex;align-items: center;">';
                if(!isset($_COOKIE['datasave'])){
                    if($item['img'] == ''){
                        $related .= '<img loading="lazy" alt="" src="../View/icon/search.webp"class="rsImg" style="width: 15px;height:15px;margin-right: 5px;">';
                    }
                    else{
                        $related .= '<img loading="lazy" alt="" src="/Controller/functions/proxy.php?q='.$item['img'].'" class="rsImg" style="filter:unset;width: 15px;height:15px;margin-right: 5px;">';
                    }
                }
                $related .= '<p>'.$item['name'].'</p></button>';
                
                if(!isset($_COOKIE['hQuery'])){
                   $related .= '</a>';
                }
                else{
                    $related .= '</form>';
                }
                
            }

          $related .= '</div>';
          if($rPrint){
          return $related;
          }
}