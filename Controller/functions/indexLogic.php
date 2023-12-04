<?php
//Homepage
if(isset($_POST['shortcutSubmit'])){
  $shURL = $_POST['shortcutURL'];
  if (strpos($shURL, "http://") !== 0 && strpos($shURL, "https://") !== 0 && strpos($shURL, "file://") !== 0) {$shURL = 'https://'.$shURL;}
  setcookie('shortcuts', $_COOKIE['shortcuts'].','. $_POST['shortcutName'].'='.$shURL, time() + 31536000, '/');
  $reload=true;
  }
  if(isset($_POST['shortcutDelete'])){
    $shortcutCookie = explode(',', urldecode($_COOKIE['shortcuts']));
    $pasteCookie = '';
    $i=0;
    foreach($shortcutCookie as &$sc){
      if($i != $_POST['shortcutID']-1){
        $pasteCookie .= $sc.',';
      }
      ++$i;
    }
    $pasteCookie = substr($pasteCookie, 0, -1);
    setcookie('shortcuts', $pasteCookie, time() + 31536000, '/');
    $reload=true;
  }
  if(isset($_POST['shortcutEdit'])){
    $shortcutCookie = explode(',', urldecode($_COOKIE['shortcuts']));

    $pasteCookie = '';
    $shURL = $_POST['shortcutURL'];
    if (strpos($shURL, "http://") !== 0 && strpos($shURL, "https://") !== 0) {$shURL = 'https://'.$shURL;}

    $i=0;
    foreach($shortcutCookie as &$sc){
      if($i != $_POST['shortcutID']-1){
        $pasteCookie .= $sc.',';
      }
      else{
        $pasteCookie .= $_POST['shortcutName'].'='.$shURL.',';
      }
      ++$i;
    }
    $pasteCookie = substr($pasteCookie, 0, -1);

    setcookie('shortcuts', $pasteCookie, time() + 31536000, '/');
    $reload=true;
  }
//Quick Settings Buttons
if(isset($_POST['revetToGlobal'])){
  setcookie('Language', 'all', time() + 604800, '/');
  setcookie('Location', 'all', time() + 604800, '/');
  $reload=true;
}
if (isset($_POST['allBut'])) {
  header("Location: ./?q=" . urlencode($_POST['q']), true);
  exit();
}
if (isset($_POST['imgBut'])) {
  header('Location: ./?image&q=' . urlencode($_POST['q']), true);
  exit();
}
if (isset($_POST['videoBut'])) {
  header('Location: ./?video&q=' . urlencode($_POST['q']), true);
  exit();
}
if (isset($_POST['newsBut'])) {
  header('Location: ./?news&q=' . urlencode($_POST['q']), true);
  exit();
}
if (isset($_POST['shopBut'])) {
  header('Location: ./?shop&q=' . urlencode($_POST['q']), true);
  exit();
}
if (isset($_POST['mapBut'])) {
  header("Location: https://www.openstreetmap.org/search?query=" . urlencode($_POST['q']));
  exit();
}

//Save Settings

  if(isset($_POST['systemTheme'])){
    setcookie('mode', null, -1, '/');
    $reload = true;
  }
  if(isset($_POST['light'])){
    setcookie('mode', '1', time() + 31536000, '/');
    $reload = true;
  }
  if(isset($_POST['dark'])){
    setcookie('mode', '2', time() + 31536000, '/');
    $reload = true;
  }

  if ($_POST['customTheme'] && $_POST['customTheme'] != "") {
    setcookie('theme', $_POST['customTheme'], time() + 31536000, '/');
  }
  if (isset($_POST['langSave'])) {
      setcookie('Language', $_POST['LangDropdown'], time() + 31536000, '/');
      $reload = true;
  }

  if (isset($_POST['tempC'])) {
    setcookie('temp', null, -1, '/');
    $reload = true;
  }
  if (isset($_POST['tempF'])) {
    setcookie('temp', 'f', time() + 31536000, '/');
    $reload = true;
  }
  if (isset($_POST['tempK'])) {
    setcookie('temp', 'k', time() + 31536000, '/');
    $reload = true;
  }
  
  if (isset($_POST['newtab'])) {
    if($_POST['newtab'] == 'newtabOff'){
    setcookie('new', 'on', time() + 31536000, '/');
    $reload = true;
    }
    else {
      setcookie('new', null, -1, '/'); 
      $reload = true;
    }
  }
  if (isset($_POST['providers'])) {
    if($_POST['providers'] == 'providersOff'){
      setcookie('providers', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('providers', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['showtime'])) {
    if($_POST['showtime'] == 'showtimeOff'){
      setcookie('showtime', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('showtime', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['datasave'])) {
    if($_POST['datasave'] == 'datasaveOff'){
      setcookie('datasave', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('datasave', null, -1, '/'); 
        $reload = true;
      }
  }
  if(isset($_POST['sugPSave'])){
      if($_POST['sugPDropdown'] == 'd'){setcookie('sugProvider', null, -1, '/');}
      else{setcookie('sugProvider', $_POST['sugPDropdown'], time() + 31536000, '/');}
    $reload = true;
  }

  if (isset($_POST['hQuery'])) {
    if($_POST['hQuery'] == 'hQueryOff'){
      setcookie('hQuery', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('hQuery', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['dSug'])) {
    if($_POST['dSug'] == 'dSugOff'){
      setcookie('DisSugges', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('DisSugges', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['dMul'])) {
    if($_POST['dMul'] == 'dMulOff'){
      setcookie('DisMul', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('DisMul', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['dQue'])) {
    if($_POST['dQue'] == 'dQueOff'){
      setcookie('DisQue', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('DisQue', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['dWid'])) {
    if($_POST['dWid'] == 'dWidOff'){
      setcookie('DisWid', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('DisWid', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['DisHImg'])) {
    if($_POST['DisHImg'] == 'DisHImgOff'){
      setcookie('DisHImg', 'on', time() + 31536000, '/');
      $reload = true;
      }
      else {
        setcookie('DisHImg', null, -1, '/');
        $reload = true;
      }
  }
  if (isset($_POST['aCou'])) {
    if($_POST['aCou'] == 'aCouOff'){
      setcookie('userid', rand(1000000, 1000000000000), time() + (86400 * 364), '/');
      setcookie('noanalytics', null, -1, '/');
      $reload = true;
    }
    else {
      setcookie('noanalytics', 'true', time() + (86400 * 364), '/');
      setcookie('userid', null, -1, '/');
      $reload = true;
    }
  }
  
if (isset($_POST['savequicksetting'])) {

  if (isset($_POST['LocDropDown'])) {
    if ($_POST['LocDropDown'] == "all") {
      setcookie('Location', 'all', time() + 31536000, '/');
    } else {
      setcookie('Location', $_POST['LocDropDown'], time() + 31536000, '/');
    }
  }

  if (isset($_POST['SafeDropDown'])) {
    if ($_POST['SafeDropDown'] == "off") {
      setcookie('safe', 'off', time() + 31536000, '/');
    } else {
      setcookie('safe', null, -1, '/');
    }
  }
  if (isset($_POST['TimeDropDown'])) {
    switch ($_POST['TimeDropDown']):

      case "day":
        setcookie('time', 'day', time() + 31536000, '/');
        break;
      case "week":
        setcookie('time', 'week', time() + 31536000, '/');
        break;
      case "month":
        setcookie('time', 'month', time() + 31536000, '/');
        break;
      case "year":
        setcookie('time', 'year', time() + 31536000, '/');
        break;
      default:
        setcookie('time', null, -1, '/');
        break;
    endswitch;
  }
  $reload = true;
}
if (isset($_POST['imgtoolsSave'])) {
  $purl = urlencode($purl);
  header('Location: ./?image&imgsize=' . $_POST['imgtoolsSize'] . '&imgcolor=' . $_POST['imgtoolsColor'] . '&imgtype=' . $_POST['imgtoolsType'] . '&imgtime=' . $_POST['imgtoolsTime'] . '&imglicence=' . $_POST['imgtoolsRights'] . '&q=' . $purl, true);
  exit();
}
if(isset($_POST['shopToolsSave'])){
  $purl = urlencode($purl);
  header('Location: ./?shop&shopMin=' . $_POST['shopPriceMin'] . '&shopMax=' . $_POST['shopPriceMax'] .'&q=' . $purl, true);
  exit();
}
if(isset($_POST['pixabayimg'])){
  $purl = urlencode($purl);
  header('Location: ./?image&pixabay&q='. $purl);
  exit();
}
if (isset($_POST['imgback'])) {
  $purl = urlencode($purl);
  header('Location: ./?image&q=' . $purl);
  exit();
}

//Reload
if ($reload) {
  $purl = urlencode($purl);

  if(strpos($url, '?image') != false){
    header("Location: ./?image&q=" . $purl, true);
    exit();
  }
  elseif(strpos($url, '?video') != false){
    header("Location: ./?video&q=" . $purl, true);
    exit();
  }
  elseif(strpos($url, '?news') != false){
    header("Location: ./?news&q=" . $purl, true);
    exit();
  }elseif(strpos($url, '?shop') != false){
    header("Location: ./?shop&q=" . $purl, true);
    exit();
  }
  else {
    header("Location: ./?q=" . $purl, true);
    exit();
  }
}
