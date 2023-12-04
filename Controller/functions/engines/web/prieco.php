<?php
function prieco($purl, $conn, $loc, $lang){
$sitelimit = false;
$siteDomain = $purl;
if (strpos($purl, 'site:') !== false){
    $sitelimit = true;
    $siteDomain = str_replace('site:', '', $purl);
}
//Get query keywords
$Pkeywords =explode(' ',mysqli_real_escape_string($conn, $siteDomain));

$purlKeywords = explode(' ', strtolower($siteDomain));

$PriEcoObj = $conn->query("SELECT * FROM `indexing` WHERE MATCH(title, description, url, H1) AGAINST('$purl' IN BOOLEAN MODE)
ORDER BY `likeable` DESC
LIMIT 1000;
");
    
    $allRes[0]='';
    $jP=0;
    $PriEcoData = array();
    $likable = array();
    $i = 0;
    $pres='';
    if ($PriEcoObj->num_rows > 0) {

        $checkFirstPriEcoObj =true;
        while ($row = $PriEcoObj->fetch_assoc()) {
            $PriEcoUrl = $row['url'];    
            
            $row['title']=utf8_encode($row['title']);
            $row['description']= utf8_encode($row['description']);   
            if(!in_array($PriEcoUrl,$allRes)){
                $allRes[$jP]=$PriEcoUrl;
                ++$jP;
            }
            else{
                continue;
            }
            if ($sitelimit && parse_url($PriEcoUrl, PHP_URL_HOST) != $siteDomain) {
                continue;   
            }
            if(!isset($_COOKIE['safe']) && $row['safeS']=='1'){
                continue;
            }
            if($PriEcoUrl[strlen($PriEcoUrl)-1] == '/'){
                $PriEcoUrl=substr_replace($PriEcoUrl ,'', -1);
            }
            ##Make PriEcoObjs##
            if (strpos($PriEcoUrl, 'https://') !== false) {
                $doma = get_string_betweens($PriEcoUrl, 'https://', '/');
            }
            $pres = '<div class="output" id="output">';
            if ($row['img'] != '' && !isset($_COOKIE['datasave'])) {
                $pres .= '<img loading="lazy" alt="‎" src="/Controller/functions/proxy.php?q=' . $row['img'] . '" class="OutSideImg">';
            }
            if (strpos($PriEcoUrl, 'https://') !== false && !isset($_COOKIE['datasave'])) {
                $pres .= '<img loading="lazy" alt="‎" class="Outfavicon" src="/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/' . $doma . '">';
            }
            $pres .= '<a ';
            if (isset($_COOKIE['new'])) {
                $pres .= 'target="_blank"';
            }
            $gurl = str_replace('/',' > ',str_replace('https://','',str_replace('http://','',str_replace('www.','', $PriEcoUrl))));
            if ( substr_compare($gurl, ' > ', -3) === 0 ) {
            $gurl = substr($gurl, 0, -3);
            }
            $pres .= 'href="' . $PriEcoUrl . '" data-sxpr-link>';
            $pres .= '<p class="OutTitle">' . $row['title'] . '</p></a>
        <p class="resLink">' . $gurl . '</p>
        <p class="snippet">' . $row['description'] . '</p>';
        if(!isset($_COOKIE['DisWid'])){
        $pres .= '<img class="filterImage sumOpen resProvider" id="sumRes" data-url="'.$PriEcoUrl.'" src="View/icon/circle-info.svg">
        <div id="sumResOut" class="sumOut snippet">
    <div id="sumImgs"></div>
    <p id="sumOut"></p>
    <p id="sumIP"></p>
    <p id="sumSSL"></p>
    <p id="sumSpeed"></p>
</div>';
        }
        if($row['tab']!=null and $row['tab'] != ''){
            $tmp = explode('<===>',$row['tab']);
            foreach($tmp as $rt){
                if(filter_var($rt, FILTER_VALIDATE_URL)){
                $pres.= '<a class="outputTab" href="'.$rt.'" ';
                if (isset($_COOKIE['new'])) {
                    $pres .= 'target="_blank"';
                }
                $pres .= '>'.parse_url($rt, PHP_URL_HOST).'</a>';
                }
            }
        }
            if (isset($_COOKIE['providers'])) {
                $pres .= '<p class="resProvider">PriEco</p>';
            }
            ##END Make PriEcoObjs##
            ##Sort by likeable##
            
//Get quality points
 $likable[$i] = $row['likeable'];
 ##
 #Keywords
 ##
 
 #Keywords in title
 $titleKeywords = explode(' ', strtolower($row['title']));
 $j = 0;
 foreach ($titleKeywords as &$tit) {
     if(in_array($tit, $purlKeywords)){
         $likable[$i]+=200;
         if($j == 0){
             $likable[$i] += 200;
         }
     }
     if($tit != null){
     ++$j;
     }
 }
 unset($titleKeywords);
 
 #Keywords in H1
 $h1Keywords = explode(' ', strtolower($row['H1']));
 foreach ($h1Keywords as &$tit) {
     if(in_array($tit, $purlKeywords)){
         $likable[$i]+=100;
     }
 }
 unset($h1Keywords);
 #Keywords in description
 $desKeywords = explode(' ', strtolower($row['description']));
 foreach ($desKeywords as &$tit) {
     if(in_array($tit, $purlKeywords)){
         $likable[$i]+=50;
     }
 }
 unset($desKeywords);
 #Keywords in url
 foreach ($purlKeywords as &$tit) {
     if (strpos($row['url'], $tit) !== false) {
         $likable[$i]+=100;
     }
 }
 //url == query
 $domain = parse_url($PriEcoUrl, PHP_URL_HOST);
 if(substr_count($domain,'.')>1) {
     $tmp = explode('.', $domain);
     $tmpNum = count($tmp);
     $domain = $tmp[$tmpNum-2].'.'.$tmp[--$tmpNum];
 }
 $domainName = explode('.', $domain)[0];
 if(strpos($PriEcoUrl,'www.')!==false) {
     $domain = 'www.'.$domain;
 }
 $schemeUrl = parse_url($PriEcoUrl)['scheme'];
 $domain = $schemeUrl.'://'.$domain;
 if($PriEcoUrl[strlen($PriEcoUrl)-1] == '/'){
     $domain.='/';
 }
 
 if ($domain == $PriEcoUrl && in_array($domainName,$purlKeywords)) {
     $likable[$i] += 1000;
 }
 
 //Same url country/language
 if(strpos($PriEcoUrl, '.'.$loc) !== false && $loc != null) {
     $likable[$i] += 50;
 }
 if(strpos($PriEcoUrl, '.'.$lang) !== false && $lang != null) {
     $likable[$i] += 50;
 }
 
 //Same website language
 if($lang == $row['lang']){
     $likable[$i]+=100;
 }
 //Same website location
 if($loc == $row['server']){
     $likable[$i]+=100;
 }
 #Keywords in masKey
 foreach ($purlKeywords as &$tit) {
     if($tit == $row['masKey']){
          $likable[$i]+=250;
          break;
      }
  }
            ##END Sort by likeable##
            $pres .= '</div>';
           $PriEcoData[$i] = $pres;
            ++$i;
            $pres = null;
        }
        array_multisort($likable, SORT_DESC, $PriEcoData);

        $splitPriEco0 = explode('class="output"',$PriEcoData[0]);
        $PriEcoData[0]='<div class="output" style="border-top-left-radius: 20px;
        border-top-right-radius: 20px;" '. $splitPriEco0[1];
    
        if(count($PriEcoData) > 10){
        $splitPriEco0 = explode('class="output"',$PriEcoData[9]);
        $PriEcoData[9]='<div class="output" style="border-bottom-left-radius: 20px;
        border-bottom-right-radius: 20px;" '. $splitPriEco0[1];
        unset($splitPriEco0);
        }
        else{
            $splitPriEco0 = explode('class="output"',$PriEcoData[count($PriEcoData)-1]);
            $PriEcoData[count($PriEcoData)-1]='<div class="output" style="border-bottom-left-radius: 20px;
            border-bottom-right-radius: 20px;" '. $splitPriEco0[1];
            unset($splitPriEco0);
        }
    }
    unset($allRes);
    return $PriEcoData;
}