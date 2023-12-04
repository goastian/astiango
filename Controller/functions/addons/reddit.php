<?php
function search_reddit($results) {
        $rPrint = false;
        $conversations = '';
        $conversations.='<p class="sectionTitle">💬 Discussions</p>

        <div class="output" style="border-radius: 20px;margin-bottom:15px;" id="output">';
        $i=0;
        foreach($results as &$res){       
            if($i >= 4){break;}
            if(!is_array($res)){break;}
            $rPrint = true;
            $conversations.='<div style="width:100%;overflow:hidden;">'; 
            $conversations.= '<div for="RedditCheck'.$i.'" class="OutSideImg"';
            if(filter_var($res['img'], FILTER_VALIDATE_URL) && !isset($_COOKIE['datasave'])) {
                $conversations .= 'style="background-image: url(/Controller/functions/proxy.php?q='. $res['img']. ');background-position: center;"';
            }
            $conversations .= '></div><div style="display:flex;width: calc(100% - 102px);">';
            if(!isset($_COOKIE['datasave'])) {
            $conversations.='<img alt="" class="Outfavicon" loading="lazy" src="/View/img/reddit.webp">  ';
            }          
           $conversations .=' <a ';
            if (isset($_COOKIE['new'])) {
                $conversations.='target="_blank"';
            }
            $conversations.= 'href="https://www.reddit.com'. $res['url']. '" style="padding-bottom:unset;">';
            $conversations.= '<p class="OutTitle" style="margin-left: 0px;width: 100%;">'.$res['title'].'</p></a></div>
            <section style="display:inline-block;color:#747684;font-size:12px;width: calc(100% - 132px);margin-left: 30px;padding:10px 0 10px 0;">r/'. $res['sub'].' ⋮ '.$res['nCom'].' 💬 ⋮ '.$res['ups'].' 🔼 ⋮ '.$res['upR']*100 .'% 👍 ⋮ Author: <p style="font-weight:bold;display:inline;">'.$res['author'].'</p></section>                
            </div>';
            ++$i;
        }
        $conversations.='</div>';
        if($rPrint){
        return $conversations;
        }
    }