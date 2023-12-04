<?php
if((($obj != '' && str_starts_with(json_encode($obj), '{"kind":"') && $searchId == 0) or (isset($g2obj) && !empty($g2obj) && $searchId == 1)) && $ImpGoogle && !isset($_COOKIE['safe']) && !isset($_COOKIE['time'])){

$purl_escaped = strtolower(mysqli_real_escape_string($conn, $purl));

if(!isset($g2obj)){$gCache = json_encode($obj);}
else{$gCache = json_encode($g2obj);}
$gCache_escaped = mysqli_real_escape_string($conn, $gCache);     

if(isset($_COOKIE['Language'])){$lang_escaped = $_COOKIE['Language'];}
else{$lang_escaped = 'all';}
if(isset($_COOKIE['Location'])){$loc_escaped = $_COOKIE['Location'];}
else{$loc_escaped = 'all';}

if(empty($g2obj) || $obj != ''){
        $sql = "INSERT INTO `googleCache`(`query`, `results`, `lang`, `loc`, `count`, `official`) VALUES ('$purl_escaped','$gCache_escaped', '$lang_escaped', '$loc_escaped', 1, 1);";
}
else{
        $sql = "INSERT INTO `googleCache`(`query`, `results`, `lang`, `loc`, `count`, `official`) VALUES ('$purl_escaped','$gCache_escaped', '$lang_escaped', '$loc_escaped', 1, 0);";
}
$conn->query($sql);

}
elseif(!$ImpGoogle && !isset($_COOKIE['safe']) && !isset($_COOKIE['time'])){
        $purl_escaped = mysqli_real_escape_string($conn, $purl);
        $conn->query("UPDATE `googleCache` SET `count` = `count` + 1 WHERE `query` = '$purl_escaped'");
}

if($simImg != ''){
        $purl_escaped = mysqli_real_escape_string($conn, $purl);
        $simI_escaped = mysqli_real_escape_string($conn, $simImg);
        $conn->query("UPDATE `suggestions` SET `img` = '$simI_escaped' WHERE `name` = '$purl_escaped'");   
}

if(!$indexW && !empty($wikiTxt)){
        $title = $conn->real_escape_string($wikiobj['title']);
        $paragraph = $conn->real_escape_string($wikiTxt);
        
        $json = [];
        foreach ($wikiobj as $name => $data) {$json[$name] = $data;}
        $infobox = $conn->real_escape_string(json_encode($json));
        
        $tmp = ($lang == 'all') ? 'en' : $lang;

        $conn->query("INSERT IGNORE INTO `wikipedia`(`title`, `paragraph`, `infobox`, `profiles`, `lang`) VALUES ('$title','$paragraph','$infobox', '[]','$tmp')");                
}

if(!$indexN && !empty($NewsObj)){
        $rows = [];
        foreach ($NewsObj as $item) {
            $title = $conn->real_escape_string($item['title']);
            $description = $conn->real_escape_string($item['description']);
            $url = $conn->real_escape_string($item['url']);
            $thumb = $conn->real_escape_string($item['img']);
            $date = $conn->real_escape_string($item['date']);
        
            $rows[] = "('$title', '$description', '$url', '$thumb', '$date')";
        }
        if (!empty($rows)) {
            $values = implode(',', $rows);
            $conn->query("INSERT IGNORE INTO `news` (`title`, `description`, `url`, `img`, `date`) VALUES $values");
        } 
}

if(!$indexY && !empty($YoutubeObj)){
$rows = [];

foreach ($YoutubeObj as $item) {
    $title = $conn->real_escape_string($item['title']);
    $description = $conn->real_escape_string($item['description']);
    $url = $conn->real_escape_string($item['url']);
    $thumb = $conn->real_escape_string($item['thumb']);
    $date = $conn->real_escape_string($item['date']);

    $rows[] = "('$title', '$description', '$url', '$thumb', '$date')";
}

if (!empty($rows)) {
    $values = implode(',', $rows);
    $conn->query("INSERT IGNORE INTO `youtube` (`title`, `description`, `url`, `thumb`, `date`) VALUES $values");
} 
}