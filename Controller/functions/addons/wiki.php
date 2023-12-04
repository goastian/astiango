<?php
function wiki($purl, $infoboxData, $wikiTxt, $ddgObj, $mysql, $hideQueryCopy)
{
    $tmp = ($_COOKIE['Language'] == 'all') ? 'en' : $_COOKIE['Language'];
    $answer = '';
    $ansImg = '';
    
    if(!isset($wikiTxt) || substr_compare($wikiTxt, 'may refer to: ', -14) === 0){return;}

    //title
    $answer .= '<div class="answer" id="answer"><a href="https://'.$tmp.'.wikipedia.org/wiki/' . str_replace('+','_',urlencode(ucwords($infoboxData['title']))) . '"';if(isset($_COOKIE['new'])){$answer.= 'target="_blank"';}$answer.=' style="color: unset;text-decoration: unset;"><h2>' . $infoboxData['title'].'</h2></a><br>';

    if(!isset($_COOKIE['datasave']) && isset($infoboxData['images'])){
        $answer .= $sum . '<a aria-label="Search for images" href="/?image&q='.urlencode($purl).'"';if(isset($_COOKIE['new'])){$answer.= 'target="_blank"';}
        $answer.='><div style="display: flex;
    flex-wrap: wrap;
    flex-direction: row;
    justify-content: space-around;
    align-items: center;
    background-color: #00000007;
    padding: 10px;
    border-radius: 20px;">';
    }
    //images
    $i=0;
    if(!isset($_COOKIE['datasave'])){
    foreach ($infoboxData['images'] as $imageUrl) {
        if($i == 2){break;}
        if($ansImg == ''){$ansImg = substr($imageUrl, 2);}
        $answer .= '<img alt="" src="/Controller/functions/proxy.php?q=' . substr($imageUrl, 2) . '"';
        if($i == 1){$answer .= 'class="filterImage"';}
        $answer .= 'style="max-width: 50%;border-radius: 30px;max-height: 200px;height:auto;width: auto;"><br>';
        ++$i;
    }
}
if(!isset($_COOKIE['datasave']) && isset($infoboxData['images'])){$answer .='</div></a>';}

//Website
 if(isset($infoboxData['Website'])){
        
    $wurl = trim(html_entity_decode($infoboxData['Website']));
    
    $answer.= '<br><a style="color: var(--linkColor);text-decoration: none;"href="https://'.$infoboxData['Website'].'"';
    if(isset($_COOKIE['new'])){$answer.= 'target="_blank"';}
    $answer .='>';
    $answer .= '🔗 '.str_replace('www.','', parse_url('https://'.$wurl)['host']);
    $answer .='</a>';
}

 //Description
 $answer.='<br><br><p style="background-color: #00000007;padding: 15px;border-radius: 20px;">' .cutString($wikiTxt,500) . 
 '<a style="color: var(--linkColor);text-decoration: none;" href="https://'.$tmp.'.wikipedia.org/wiki/' . str_replace('+','_',urlencode(ucwords($infoboxData['title']))). '"';if(isset($_COOKIE['new'])){$answer.= 'target="_blank"';}$answer.='>Wikipedia</a>
 </p><br>
 <div style="display: flex;padding-left: 10px;padding-right: 10px;">';

//Summarized
if(isset($wikiTxt) && !$dev){
//$summary = summarizeText($wikiTxt, 2);
    $wikiTxt = substr($wikiTxt,0, 600);
    foreach($summary as &$su){
      $Tsum .= ' '.$su;
    }
    if(strlen($Tsum) >= 200 && strlen($Tsum) <= 850){
    $answer .= '<input type="checkbox" id="sumMoreCheck" style="display:none">
    <label class="sumMore" for="sumMoreCheck" style="margin-right: 10px;"><p><b style="font-size: 13px;">Summarized</b></p><p style="margin-top:10px;font-size:13px;">'.$Tsum.'</p></label>';
    $loaded[0] = true;
    }
}

    //Infobox
    $answer .= '<input type="checkbox" id="wikiMoreCheck" style="display:none">
    <label class="wikiMore" for="wikiMoreCheck"><p><b style="font-size: 13px;">Infobox</b></p>';
foreach ($infoboxData as $name => $data) {
    if($name == 'images'){continue;}
    $answer .= '<div style="margin-top:10px;font-size:13px;"><p style="font-weight: bold;font-size: 12px;">'.$name . '</p><p>' . $data . '</p></div>';
} 
$answer .= '</label></div>';

    if($answer!=''){
        $answer .= '<p style="font-weight: bold;font-size: 12px;">Profiles</p>';
        $answer .= '<a href="https://'.$tmp.'.wikipedia.org/wiki/' . $infoboxData['title'].'"'; if (isset($_COOKIE['new'])) {
            $answer .= 'target="_blank"';
        } $answer .='><button class="socialBtn"><div>';
        if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/wiki.svg" class="profileIcon">';}
        $answer .= '<p>Wikipedia</p></div></button></a>';

        $twitter='';
        $facebook='';
        $imdb='';
        $tomato='';
        $spotify='';
        $apple='';
        
        if(is_string($ddgObj)){
        $ddgObj=json_decode($ddgObj,true);
        $twitter = $ddgObj['Tiwtter'];
        $facebook = $ddgObj['Facebook'];
        $imdb = $ddgObj['IMDb'];
        $tomato = $ddgObj['Tomatoes'];
        $spotify = $ddgObj['Spotify'];
        $apple = $ddgObj['Apple'];
        }

        if($twitter != ''){
            $answer .= '<a href="https://twitter.com/'.$twitter.'"'; if (isset($_COOKIE['new'])) {
                $answer .= 'target="_blank"';
            } $answer .='><button class="socialBtn"><div>';
            if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/twitterlogo.svg" class="profileIcon">';}
            $answer .= '<p>Twitter</p></div></button></a>';
        }

        if($facebook != ''){
            $answer .= '<a href="https://www.facebook.com/'.$facebook.'"'; if (isset($_COOKIE['new'])) {
                $answer .= 'target="_blank"';
            } $answer .='><button class="socialBtn"><div>';
            if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/facebook.svg" class="profileIcon">';}
            $answer .= '<p>Facebook</p></div></button></a>';
        }
            
        if($imdb != ''){
            $answer .= '<a href="https://www.imdb.com/name/'.$imdb.'"'; if (isset($_COOKIE['new'])) {
                $answer .= 'target="_blank"';
            } $answer .='><button class="socialBtn"><div>';
            if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/imdb.svg" class="profileIcon">';}
            $answer .= '<p>IMDb</p></div></button></a>';
        }
        if($tomato != ''){
            $answer .= '<a href="https://www.rottentomatoes.com/'.$tomato.'"'; if (isset($_COOKIE['new'])) {
                $answer .= 'target="_blank"';
            } $answer .='><button class="socialBtn"><div>';
            if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/tomato.svg" class="profileIcon">';}
            $answer .= '<p>Rotten Tomatoes</p></div></button></a>';
            }

        if($spotify != ''){
            $answer .= '<a href="https://open.spotify.com/artist/'.$spotify.'"'; if (isset($_COOKIE['new'])) {
                $answer .= 'target="_blank"';
            } $answer .='><button class="socialBtn"><div>';
            if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/spotify.svg" class="profileIcon">';}
            $answer .= '<p>Spotify</p></div></button></a>';
        }

        if($apple != ''){
            $answer .= '<a href="https://music.apple.com/artist/'.$apple.'"'; if (isset($_COOKIE['new'])) {
                $answer .= 'target="_blank"';
            } $answer .='><button class="socialBtn"><div>';
            if(!isset($_COOKIE['datasave'])) {$answer.='<img alt="‎" src="./View/icon/profiles/apple.svg" class="profileIcon">';}
            $answer .= '<p>Apple Music</p></div></button></a>';
        }
    
    $answer .= '<br><br>'. $hideQueryCopy .'</div>';// Place for ad after <br>
   
    $ret[] = $answer;
    $ret[1] = $ansImg;
    return $ret;
    }
}