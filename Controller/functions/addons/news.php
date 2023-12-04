<?php
function search_news($NewsObj){
    $i =0;    
    $rPrint = false;
    $news = '<a href="/?news&q='.urlencode($_GET['q']).'"><p class="sectionTitle">🗞️ News</p></a>
    
    <div class="output" style="border-radius: 20px;margin-bottom:15px;background:none;
    display:flex;overflow:auto hidden; height:300px;" id="output">';
    foreach ($NewsObj as &$item) {  
        $rPrint = true;
        if($i>6){break;}
        $domain = str_replace('www.','', parse_url($item['url'])['host']);

        $news .= '
                    <div class="imgoutdiv" style="width:auto !important;min-width:unset;margin-right:10px;padding:0;">
                    <a href="'.$item['url'].'"'; 
                    if (isset($_COOKIE['new'])) {
                        $news .=  'target="_blank"';
                    }
                    $news .= '>
                    <button title="News button" class="ytvideobtn"';
            if(!isset($_COOKIE['datasave'])) {
                $news .= 'style="background-image: url(Controller/functions/img_proxy.php?q='.urlencode($item['img']).');"';
            }
            $news .= '></button>
            <div class="imgoutlink videossearch">
              <div style="display: flex;align-items: center;padding: 3px;flex-direction: row;justify-content: space-between;">
                <div style="display:flex;align-items: center;">';
                if(!isset($_COOKIE['datasave'])) {
                  $news .= '<img alt="" style="width: 20px;height: 20px;border-radius: 20px;"src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'. get_string_betweens($item['url'], 'https://', '/').'">';
                }
                $news .= '<p style="font-size:10px;padding-left:5px;">'.$domain.'</p></div>
                <p style="font-size:10px;padding-right:5px;">';
                $currentDate = new DateTime();
                $specifiedDate = new DateTime($item['date']);
                $news .=$currentDate->diff($specifiedDate)->format('%a').' days ago</p>
              </div>
                <p class="ytTitle">'.substr($item['title'], 0, 47).'...</p>
        <p style="font-size:10px;padding: 0 5px 0px 5px;
        display: -webkit-box;
        -webkit-line-clamp: 3;
        line-height:14px;
        -webkit-box-orient: vertical;
        overflow: hidden;">'.substr(strip_tags($item['description']), 0, 120) . '...</p>
        </div>
        </a>
        </div>
              ';
              ++$i;
                }
                $news .= '<div class="imgoutdiv" style="width:auto !important;min-width:unset;margin-right:10px;padding:0;">
                <a href="/?news&q='.urlencode($_GET['q']).'">
                 <div class="videossearch" style="display:block;cursor:pointer;width: 100px;height: 100px;margin-top: 75px;border-radius: 2000px;"><img class="filterImage" src="View/icon/arrow-right.svg" style="width: 40%;height: 100%;"></div>
                 </a>      
                 </div>';
              $news .= '</div>';
    if($rPrint){
    return $news;
    }
}