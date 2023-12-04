<?php
ini_set('session.cookie_httponly', 1);
ini_set('session.cookie_domain', '.jojoyou.org');
session_start();
include 'Controller/simple_html_dom.php';

$gTime = microtime(true);

//Development mode (Get search results from json files in ./Controller/dev folder)
$dev = false;
//CSS version
$cssver = 125;
//Variable, controls reloading on settings change
$reload = false;

//Values
$onion = 'http://priecovk7jsuh3tvkh62c6j4oep3l5bldigpzmay26rdpqz357t5dmad.onion/';
function removeDisFile($name, $time = 3600){
  if(file_exists($name)){
    $tmp = file_get_contents($name);
    if(time()>$tmp+$time){
      unlink($name);
    }
  }
}
$sumPath = '';

removeDisFile('disGoogle.txt');
removeDisFile('disGoogle2.txt');
removeDisFile('disBing.txt', 86400);
removeDisFile('disBing2.txt');
removeDisFile('disBrave.txt');
removeDisFile('disMojeek.txt');

  //Get data from $PromoFile
  $promoobj = json_decode(file_get_contents('./Controller/value/data.json'), true);

//Suggest tor
/*
if (strpos(file_get_contents('Controller/value/tor.txt'), $_SERVER['REMOTE_ADDR']) !== false) {
  echo '<div style="position: fixed;
  right: 0;
  background-color: red;
  padding: 5px;
  border-radius: 0 0 0 20px;
  "><p>Using TOR?<br>Consider using our onion PriEco.</p><button>No</button></div>';
}
*/
//Prepare for search request with search engine APIs
include './Controller/database.php';
include 'Controller/functions/func.php';
//Function to get string between characters
include 'Controller/functions/getsearch.php';

//Check for img search and for empty query
$urlSet = explode('&', $urlSet);
for ($i = 1; $i < count($urlSet); $i++) {
  if(strpos($urlSet[$i], 'q=') !== false && !isset($_COOKIE['hQuery'])){
    $purl = urldecode(str_replace('q=', '', $urlSet[$i]));
  }
  if(!isset($_COOKIE['hQuery'])){
  if ($urlSet[$i] == 'image') {
    $type = 'image';
  }
  if ($urlSet[$i] == 'video') {
    $type = 'video';
  }
  if ($urlSet[$i] == 'news') {
    $type = 'news';
  }
  if(isset($_GET['shop'])){
    $type='shop';
  }
  }

 
  
  if(strpos($urlSet[$i], 'page=') !== false){
    $page = str_replace('page=', '', $urlSet[$i]);
  }
  if (strpos($urlSet[$i], 'imgsize=') !== false) {
    $imgsize = str_replace('imgsize=', '', $urlSet[$i]);
  }
  if (strpos($urlSet[$i], 'imgcolor=') !== false) {
    $imgcolor = str_replace('imgcolor=', '', $urlSet[$i]);
  }
  if (strpos($urlSet[$i], 'imgtype=') !== false) {
    $imgtype = str_replace('imgtype=', '', $urlSet[$i]);
  }
  if (strpos($urlSet[$i], 'imgtime=') !== false) {
    $imgtime = str_replace('imgtime=', '', $urlSet[$i]);
  }
  if (strpos($urlSet[$i], 'imglicence=') !== false) {
    $imgright = str_replace('imglicence=', '', $urlSet[$i]);
  }
  if (strpos($urlSet[$i], 'pixabay') !== false) {
    $pixabay = true;
  }
}

$page = isset($page) ? $page : 0;

if ((isset($_POST['q']) && $_POST['q'] != $purl) && !isset($_COOKIE['hQuery'])) {
  header('Location: ./?' . $type . '&q=' . urlencode($_POST['q']));
  exit();
}

##
#Protection
##
if(!$dev){
  include 'Controller/functions/protection/cookie.php';
  include 'Controller/functions/protection/shield.php';}
  
//Bangs
if(strpos($purl, '!') !== false){
  $bangObj = json_decode(file_get_contents('Controller/value/bangs.json'), true);
  $tmp = explode(' ', $purl);
  
  foreach($tmp as &$t){
    $tmp2 = str_replace('!','',$t);
    if(isset($bangObj['bangs'][$tmp2]) && strpos($t, '!') !== false){
      $tmps = str_replace($t,'', $purl);
      
      if(isset($_COOKIE['DisMul'])){header('Location: ' . $bangObj['bangs'][$tmp2] . urldecode($tmps));exit();}
      else{$bangs[]=$bangObj['bangs'][$tmp2];}
    }
    else{
      $bangQuery .= $t.' ';
    }
  }

  if(!isset($_COOKIE['DisMul'])){
    $bangQuery = urldecode(substr($bangQuery, 0, -1));

    $bangOne = '';
    $bangScript = '<script>';
    $i=0;
   foreach($bangs as &$ban){
    if($i == 0){
      $bangOne = '<script>setTimeout(function(){ window.location.href = "'.$ban.$bangQuery.'"; }, 7000);</script>';
    }
    else{
      $bangScript .= 'window.open("'.$ban.$bangQuery.'", "_blank");';
    }
    ++$i;
   }

   $bangScript .= '</script>';
   if($bangScript != '<script></script>'){
  echo $bangScript,'<br>
  <p>Redirecting to first bang in 7 seconds...<br>
  If multi bang didn',"'",'t work, allow it in you browser.</p>',
  $bangOne;
   }
   else{
    header("Location: " . $ban.$bangQuery, true);
   }
  exit();
  }
}
    //Set parameters for search request
    $lang = $loc = 'all';
    $msgLang = $msgLoc = false;

    if (isset($_COOKIE['Language'])) {
      $lang = $_COOKIE['Language'];
    }
    if (isset($_COOKIE['Location'])) {
      $loc = $_COOKIE['Location'];
    }
    
    if (!isset($_COOKIE['Location']) || !isset($_COOKIE['Language'])) {
     if(!$dev && !str_starts_with(getenv('REMOTE_ADDR'), '192.') && !str_starts_with(getenv('REMOTE_ADDR'), '127.')){
    $ch = curl_init();

    curl_setopt($ch, CURLOPT_RETURNTRANSFER, 1);

     curl_setopt($ch, CURLOPT_URL, 'https://jojoyou.org/ipAPI/?ip='.getenv('REMOTE_ADDR'));
    curl_setopt($ch, CURLOPT_USERAGENT, 'Mozilla/5.0 (Windows; U; Windows NT 5.1; en-US) AppleWebKit/525.13 (KHTML, like Gecko) Chrome/0.A.B.C Safari/525.13');

    $geo =json_decode(curl_exec($ch), true);
      if(!isset($_COOKIE['Location'])){
        if($geo['code'] != ''){
        setcookie('Location', $geo['code'].'_3', time() + 604800, '/');
        }
        else{
          setcookie('Location', 'all', time() + 604800, '/');
        }
      }
      if(!isset($_COOKIE['Language'])){
        if($geo['lang'] != ''){
          setcookie('Language', $geo['lang'].'_3', time() + 604800, '/');
        }
          else{
            setcookie('Language', 'all', time() + 604800, '/');
          }
      }
      $reload = true;
    }
    else{
      setcookie('Language', 'all', time() + 604800, '/');
      setcookie('Location', 'all', time() + 604800, '/');
      $reload=true;
    }
    }
    
    if(strpos($lang,'_') !== false){
      $lang = explode('_',$lang);$langNum = $lang[1]-1;$lang=$lang[0];
      if($langNum == 0){setcookie('Language', $lang, time() + 604800, '/');}
      else{setcookie('Language', $lang.'_'.$langNum, time() + 604800, '/');}
      $msgLang = true;
    }
    if(strpos($loc,'_') !== false){
      $loc = explode('_',$loc);$locNum = $loc[1]-1;$loc = $loc[0];
      if($langNum == 0){setcookie('Location', $loc, time() + 604800, '/');}
      else{setcookie('Location', $loc.'_'.$locNum, time() + 604800, '/');}
      
      $msgLoc = true;
    }

    if($msgLang or $msgLoc){
      echo '<div style="display:flex;flex-wrap:wrap;position:fixed;bottom: 0;right: 0;backdrop-filter: blur(5px);padding: 5px;border-radius: 20px 20px 0 0;border: solid 1px gray;">
        <p>Location and language have been set. </p>
        <form method="post">
        <input type="submit" name="revetToGlobal" value="Revert to global" class="revertLocLang" style="cursor:pointer;margin-left: 10px;background: none;border: none;font-size: 16px;">
        </form>
      </div>';
    }


    $fahrenheitCountries = ['as', 'bs', 'bz', 'ca', 'fm', 'gu', 'mh', 'mp', 'pr', 'pw', 'tc', 'us', 'vi'];
    if(!isset($_COOKIE['temp']) && in_array($_COOKIE['Location'], $fahrenheitCountries)){setcookie('temp', 'f', time() + 604800, '/');}

    if(isset($_COOKIE['safe'])){$safe = $_COOKIE['safe'];}
    else{$safe = 'active';}

    if(isset($_COOKIE['new'])){$new = $_COOKIE['new'];}
    else{$new = 'off';}

    if (!isset($imgsize)){$imgsize = 'all';}
    if (!isset($imgcolor)){$imgcolor = 'all';}
    if (!isset($imgtime)){$imgtime = 'all';}
    if (!isset($imgtype)){$imgtype = 'all';}
    if (!isset($imgright)){$imgright = 'all';}

    if(isset($_COOKIE['time'])){
    switch ($_COOKIE['time']):
      case 'day':
        $date = 'd1';
        break;
      case 'week':
        $date = 'w1';
        break;
      case 'month':
        $date = 'm1';
        break;
      case 'year':
        $date = 'y1';
        break;
    endswitch;
  }
  else{
    $date = '';
  }
    include 'Model/header.php';
##
#Analytics
##
if(!$dev){include('Controller/functions/analytics/analytics.php');}

#IndexLogic
include 'Controller/functions/indexLogic.php';

//Get style.css file and post custom settings for light and dark mode
  include 'Model/style.php';

  
  if ($purl != null or $purl != '') {
  include 'Controller/retrieve.php';
  //indexLogic is here for saving settings
    //Get user from cookie and compare it with database
    if (isset($_COOKIE['auth']) && !$dev) {  
      $auth = '▛' . $auth;
  
      $possibleUsr = strstr($auth, '▛');
      $possibleUsr = strstr($auth, ' ', true);
      $possibleUsr = substr($possibleUsr, 3);
  
      $authName = str_replace('▛' . $possibleUsr . ' ', '', $auth);
  
  
      $sql = "SELECT * FROM JYS WHERE Username='$possibleUsr'";
      $result = $conn->query($sql);
      if ($result->num_rows > 0) {
        $row = $result->fetch_assoc();
        $authDB = explode(',', $row['Auth']);
        foreach ($authDB as $a) {
          if ($a == $authName) {
            $usr = $row['Username'];
            $usrSearches = $row['Searches'];
            $_SESSION['usr'] = $usr;
            continue;
          }
        }
      }
      $usr = $row['Username'];
      $select_user = "SELECT * FROM JYS WHERE Username='$usr'";
      $run_qry = mysqli_query($conn, $select_user);
      if (mysqli_num_rows($run_qry) > 0) {
        if ($row = mysqli_fetch_assoc($run_qry)) {
          $usrSearches = $row['Searches'];
          $usrSearches++;
          $update_user = "UPDATE JYS SET Searches='$usrSearches' WHERE Username='$_SESSION[usr]'";
          $run_qry = mysqli_query($conn, $update_user);
        }
      }
    }
    if(!isset($usr)){
      $usr = 'Guest';
    }
  //Create inputbox with settings
  include 'Model/searchbox.php';
  //Print output
  $priecoTime = -1;
  $resultTime = microtime(true);
  include 'Controller/functions/output.php';  
  $resultTime = microtime(true) - $resultTime;
  //Improve PriEco
  if(!$dev){include 'Controller/functions/analytics/improve.php';}
  //Print footer, contains JS for Ads
  include 'Model/footer.php';
  
  if(isset($_COOKIE['showtime'])){
    echo '<div class="showtime">
    <h4>Time to load:</h4>
    <p>Full time: ', (microtime(true) - $gTime),' s<br>
    Together results time: ', $resultTime ,' s<br>
    PriEco results time: ', $priecoTime ,' s
    </p></div>';
  }

  //Increase daily searches
  if(!$dev){
  $currentDate = date('Y-m-d');
  $sql = "SELECT * FROM users WHERE day = '$currentDate'";
  $result = $conn->query($sql);
  
  if ($result->num_rows > 0) {
      $sql = "UPDATE users SET searches = searches + 1 WHERE day = '$currentDate'";
      $conn->query($sql);
  } else {
      $sql = "INSERT INTO users (day, searches) VALUES ('$currentDate', 1)";  
      $conn->query($sql);
  }
  }
} else {  
  //Print mainpage
  include 'Model/mainpage.php';
    //Print footer, contains JS for Ads
    include 'Model/footer.php';
}
?>
