<?php
function youtube($YoutubeObj){
    $i =0;    
    $rPrint = false;

    $yt = '<a href="/?video&q='.urlencode($_GET['q']).'"><p class="sectionTitle">📷 Videos</p></a>
    
    <div class="output" style="border-radius: 20px;margin-bottom:15px;background:none;
    display:flex;overflow:auto hidden; height:300px;" id="output">';
    foreach ($YoutubeObj as &$item) {  
        if($i>6){break;}   
        $rPrint=true;
        $yt .= '
                    <div class="imgoutdiv" style="width:auto !important;min-width:unset;margin-right:10px;padding:0;">
                    <a href="https://www.youtube.com/watch?v=' . $item['url'] . '"'; 
                    if (isset($_COOKIE['new'])) {
                        $yt .=  'target="_blank"';
                    }
                    $yt .= '>
                    <button title="YouTube video button" class="ytvideobtn"';
            if(!isset($_COOKIE['datasave'])) {
                $yt .= 'style="background-image: url(/Controller/functions/proxy.php?q=https://i.ytimg.com/vi/'.$item['thumb'].');"';
            }
            $yt .= '></button>
            <div class="imgoutlink videossearch">
              <div style="display: flex;align-items: center;padding: 3px;flex-direction: row;justify-content: space-between;">
                <div style="display:flex;align-items: center;">';
                if(!isset($_COOKIE['datasave'])) {
                  $yt .= '<img alt="" style="width: 20px;height: 20px;border-radius: 20px;"src="View/icon/profiles/youtube.svg">';
                }
                $yt .= '<p style="font-size:10px;padding-left:5px;">YouTube</p></div>
                <p style="font-size:10px;padding-right:5px;">';
                $currentDate = new DateTime();
                $specifiedDate = new DateTime($item['date']);
                $yt .=$currentDate->diff($specifiedDate)->format('%a').' days ago</p>
              </div>
                <p class="ytTitle">'.$item['title'].'</p>
        <p style="font-size:10px;padding: 0 5px 0px 5px;
        display: -webkit-box;
        -webkit-line-clamp: 3;
        line-height:14px;
        -webkit-box-orient: vertical;
        overflow: hidden;">'.strip_tags($item['description']).'</p>
        </div>
        </a>
        </div>
              ';
              ++$i;
                }
                $yt .= '<div class="imgoutdiv" style="width:auto !important;min-width:unset;margin-right:10px;padding:0;">
                <a href="/?video&q='.urlencode($_GET['q']).'">
                 <div class="videossearch" style="display:block;cursor:pointer;width: 100px;height: 100px;margin-top: 75px;border-radius: 2000px;"><img class="filterImage" src="View/icon/arrow-right.svg" style="width: 40%;height: 100%;"></div>
                 </a>      
                 </div>';
              $yt .= '</div>';
              if($rPrint){
                return $yt;
                }
}