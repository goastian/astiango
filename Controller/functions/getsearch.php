<?php

//Get url
if(isset($_SERVER['HTTPS']) && $_SERVER['HTTPS'] === 'on')
     $url = "https://";
else
     $url = "http://";
$url.= $_SERVER['HTTP_HOST'];
$url.= $_SERVER['REQUEST_URI'];
$url .= "▛";

//Get query from url
function get_string_between($string, $start, $end){
    $string = ' ' . $string;
    $ini = strpos($string, $start);
    if ($ini == 0) return '';
    $ini += strlen($start);
    $len = strpos($string, $end, $ini) - $ini;
    return substr($string, $ini, $len);
}

if(!isset($_COOKIE['hQuery'])){
$purl = get_string_between($url, '?', '▛');
$type = '';
}
else{
    if(get_string_between($url, '?', '▛') != null){
        if(!isset($_POST['q'])){$_SESSION['query'] = get_string_between($url, 'q=', '▛');}
        else{$_SESSION['query'] = $_POST['q'];}
        header('Location: /');
        exit();
    }
    elseif(isset($_POST['q'])){
    $purl = $_POST['q'];
    $type = ($_POST['search_type']?? null);
    $page = (int)($_POST['page'] ?? null);
    }
    elseif(isset($_SESSION['query'])){
        $purl = $_SESSION['query'];
        $type = 'all';
        $page = 0;
    }
}
$_SESSION['query'] = $purl;
$purl =  urldecode($purl);
$urlSet = '&'.$purl;