<?php
header("Content-type: application/json; charset=utf-8");
if(!isset($_GET['q'])){
    echo '["No Query!"]';
    return;
}
if(!isset($_GET['api'])){
    echo '["No API key!"]';
    return;
}
$dev = false;
include '../Controller/database.php';
if($_GET['api'] != $_ENV['Obunic'] && $_GET['api'] != $_ENV['Artado']){
    echo '["Incorrect API key!"]'; 
    return;
}

$resNum = isset($_GET['num']) ? $_GET['num'] : 10;
$loc = isset($_GET['loc']) ? $_GET['loc'] : 'all';
$lang = isset($_GET['lang']) ? $_GET['lang'] : 'all';

$siteDomain = $purl = $_GET['q'];
$purl = urldecode($purl);
$sitelimit = false;
if (strpos($purl, 'site:') !== false){
    $sitelimit = true;
    $siteDomain = str_replace('site:', '', $purl);
}

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
            if(!isset($_GET['safe']) && $row['safeS']=='1'){
                continue;
            }
            if($PriEcoUrl[strlen($PriEcoUrl)-1] == '/'){
                $PriEcoUrl=substr_replace($PriEcoUrl ,'', -1);
            }

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
           $PriEcoData[$i] = json_encode($row);
            ++$i;
            $pres = null;
        }
        array_multisort($likable, SORT_DESC, $PriEcoData);
    }
    
    $res = array();
    if($resNum>count($PriEcoData)){$resNum=count($PriEcoData);}
  
    for ($i = 0; $i < $resNum; $i++) {
        $result = json_decode($PriEcoData[$i], true);
        if($result['title']==null){continue;}
        $formattedResult = array(
            'title' => $result['title'],
            'description' => $result['description'],
            'url' => $result['url'],
            'lang' => $result['lang'],
            'loc' => $result['server'],
            'tab' => $result['tab'],
            'safeS' => $result['safeS'],
            'img' => $result['img'],
            'date' => $result['date']
        );
        $res[] = $formattedResult;
    }
    
    echo json_encode($res);
    