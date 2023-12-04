<?php 
echo'<div class="wrapper">
<input type="checkbox" id="btn" hidden /><label
  for="btn"
  class="menu-btn"
  style="width: 20px;height: 20px;cursor:pointer;"
  ><img
    alt="icnSet"
    class="icnSet"
    src="./View/icon/gear.svg"
    style="width: 15px; height: 15px;"
/></label>
<nav id="sidebar">
  <label for="btn" class="close-menu-btn"
    ><img style="cursor:pointer;position: absolute; width:35px;padding:10px;right: 15px;top: 5px;"
    alt="icnCross" src="./View/icon/cross.svg"></label
  >
  <div class="title"><img alt="" style="width: 15px;
  height: 15px;
  margin-right: 5px;
  margin-left: 5px;"src="./View/icon/gear.svg">Settings</div>
  <div class="list-items">
    <form method="post" class="form-items" action="">
      <div class="setGroup">
      <div style="border-bottom: solid 2px grey;" class="setListItem">
        <h3>General</h3>
      </div>
      <br />
      <div class="setListItem">
        <p>Language:</p>
        <div>
        <select
          style="border: none;padding: 10px;border-radius: 6px;"
          name="LangDropdown"
        >
          <option disabled hidden selected>
';
if (isset($lang) && $lang !== 'all')
{
    $i = array_search($lang, $langVal);
    echo $langName[$i];
}
echo '</option><option value="all"';
if (!isset($lang) or $lang == 'all')
{
    echo 'selected';
}
echo '>All languages</option>
<option name="ar" value="ar">Arabic</option>
<option name="bg" value="bg">Bulgarian</option>
<option name="ca" value="ca">Catalan</option>
<option name="hr" value="hr">Croatia</option>
<option name="cs" value="cs">Czech</option>
<option name="da" value="da">Danish</option>
<option name="nl" value="nl">Dutch</option>
<option name="en" value="en">English</option>
<option name="et" value="et">Estonian</option>
<option name="fi" value="fi">Finnish</option>
<option name="fr" value="fr">French</option>
<option name="de" value="de">German</option>
<option name="el" value="el">Greek</option>
<option name="iw" value="iw">Hebrew</option>
<option name="hu" value="hu">Hungarian</option>
<option name="is" value="is">Icelandic</option>
<option name="id" value="id">Indonesian</option>
<option name="it" value="it">Italian</option>
<option name="ja" value="ja">Japanese</option>
<option name="ko" value="ko">Korean</option>
<option name="lv" value="lv">Latvian</option>
<option name="lt" value="lt">Lithuanian</option>
<option name="no" value="no">Norwegian</option>
<option name="pl" value="pl">Polish</option>
<option name="pt" value="pt">Portuguese</option>
<option name="ro" value="ro">Romanian</option>
<option name="ru" value="ru">Russian</option>
<option name="sr" value="sr">Serbian</option>
<option name="sk" value="sk">Slovak</option>
<option name="sl" value="sl">Slovenian</option>
<option name="es" value="es">Spanish</option>
<option name="sv" value="sv">Swedish</option>
<option name="tr" value="tr">Turkish</option>
</select> <input type="submit" value="Save" class="langSave" name="langSave"></div></div><br><div class="setListItem"><label for="newtab">Open links in New tab:</label><label class="switch">
<input type="submit" name="newtab" id="newtab" value="newtab';if (isset($_COOKIE['new'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (isset($_COOKIE['new']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div><br><div class="setListItem"><p>Theme:</p>
<div class="themeMenu" style="padding: 10px;padding-bottom:0;border-radius: 20px;">
<div style="width: 100%;display: flex;justify-content: space-between;">


<input type="radio" style="display:none;"';
if(!isset($_COOKIE['mode'])){
echo ' checked ';
}
echo '>
<input type="submit" id="systemTheme" name="systemTheme" style="display:none;">
<label class="';if(!isset($_COOKIE['mode'])){echo 'setChooseChecked ';}echo 'unitTemp" for="systemTheme">🌓</label>

<input type="submit" id="light" name="light" style="display:none;">
<label class="';if(isset($_COOKIE['mode']) && $_COOKIE['mode'] == 1){echo 'setChooseChecked ';}echo 'unitTemp" for="light">☀️</label>

<input type="submit" id="dark" name="dark" style="display:none;">
<label class="';if(isset($_COOKIE['mode']) && $_COOKIE['mode'] == 2){echo 'setChooseChecked ';}echo 'unitTemp" for="dark">🌑</label>

<label class="labCustom" for="custom">🌈</label></div>
<input type="checkbox" class="showCustomThemeBox" aria-label="Open custom editor" style="float:right;"><textarea name="customTheme" class="';
if (isset($_COOKIE['mode']) && $_COOKIE['mode'] == 3)
{
    echo 'CustomThemeShow';
}
echo 'CustomThemeBox" placeholder="Custom theme(Css injection)">';
if (isset($_COOKIE['theme']))
{
    echo $_COOKIE['theme'];
}
echo '</textarea>
<div style="display:grid;grid-template-columns: auto auto auto;text-align:center;width:calc(100% - 5px);">
<label for="systemTheme" style="font-size:12px;margin-bottom: 5px;cursor:pointer;">System</label>
<label for="light" style="font-size:12px;cursor:pointer;">Light</label>
<label for="dark" style="font-size:12px;cursor:pointer;">Dark</label>
</div>

</div>
</div>

<br><div class="setListItem"><p>Units:</p>
<div class="themeMenu" style="padding: 10px;border-radius: 20px;">
<div style="width: 100%;display: flex;justify-content: space-between;">


<input type="submit" id="tempC" name="tempC" style="display:none;">
<label class="';if(!isset($_COOKIE['temp'])){echo 'setChooseChecked ';}echo 'unitC unitTemp" for="tempC">°C</label>

<input type="submit" id="tempF" name="tempF" style="display:none;">
<label class="';if(isset($_COOKIE['temp']) && $_COOKIE['temp'] == 'f'){echo 'setChooseChecked ';}echo 'unitF unitTemp" for="tempF">°F</label>

<input type="submit" id="tempK" name="tempK" style="display:none;">
<label class="';if(isset($_COOKIE['temp']) && $_COOKIE['temp'] == 'k'){echo 'setChooseChecked ';}echo 'unitK unitTemp" for="tempK">K</label>
</div>


</div>
</div>

<br><div class="setListItem"><label for="datasaver">Cellular data saver:</label><label class="switch">
<input type="submit" style="display:none;" name="datasave" id="datasaver" value="datasave';if (isset($_COOKIE['datasave'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (isset($_COOKIE['datasave']))
{
    echo 'checked';
}
echo '><span class="slider round"></span>
</label></div>

<br><div class="setListItem"><p>Suggestions:</p>
<div>
<select
  style="border: none;padding: 10px;border-radius: 6px;"
  name="sugPDropdown">

<option value="d"';if (!isset($_COOKIE['sugProvider'])){echo 'selected';}echo '>DuckDuckGo</option>
<option value="g"';if (isset($_COOKIE['sugProvider']) && $_COOKIE['sugProvider'] == 'g'){echo 'selected';}echo '>Google</option>
<option value="p"';if (isset($_COOKIE['sugProvider']) && $_COOKIE['sugProvider'] == 'p'){echo 'selected';}echo '>PriEco</option>
</select> <input type="submit" value="Save" class="langSave" name="sugPSave">
</div>
</div>


</div>
<div class="setGroup"><div style="border-bottom: solid 2px grey;"class="setListItem"><h3>Data</h3></div>
<br>
<div class="setListItem"><label for="aCou"><p>User counter:</p><p style="font-size:12px;">Anonymous ping</p></label><label class="switch">
<input type="submit" style="display:none;" name="aCou" id="aCou" value="aCou';if (isset($_COOKIE['userid'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (isset($_COOKIE['userid']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>
</div>

<div class="setGroup"><div style="border-bottom: solid 2px grey;"class="setListItem"><h3>Privacy</h3></div>
<br>
<div class="setListItem"><label for="hQuery">Hide query:</label><label class="switch">
<input type="submit" style="display:none;" name="hQuery" id="hQuery" value="hQuery';if (isset($_COOKIE['hQuery'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (isset($_COOKIE['hQuery']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>
</div>

<div class="setGroup"><div style="border-bottom: solid 2px grey;"class="setListItem"><h3>JavaScript</h3></div>
<br>
<div class="setListItem"><label for="dSug">Suggestions:</label><label class="switch">
<input type="submit" style="display:none;" name="dSug" id="dSug" value="dSug';if (isset($_COOKIE['DisSugges'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (!isset($_COOKIE['DisSugges']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div><br>
<div class="setListItem"><label for="dSug">Multi Bang:</label><label class="switch">
<input type="submit" style="display:none;" name="dMul" id="dMul" value="dMul';if (isset($_COOKIE['DisMul'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (!isset($_COOKIE['DisMul']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div><br>
<div class="setListItem"><label for="dQue">Clear Query:</label><label class="switch">
<input type="submit"style="display:none;" name="dQue" id="dQue" value="dQue';if (isset($_COOKIE['DisQue'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (!isset($_COOKIE['DisQue']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>
<br><div class="setListItem"><label for="dWid">Widgets:</label><label class="switch">
<input type="submit" style="display:none;" name="dWid" id="dWid" value="dWid';if (isset($_COOKIE['DisWid'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (!isset($_COOKIE['DisWid']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>

<br><div class="setListItem"><label for="DisHImg">High res img:</label><label class="switch">
<input type="submit" style="display:none;" name="DisHImg" id="DisHImg" value="DisHImg';if (isset($_COOKIE['DisHImg'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (!isset($_COOKIE['DisHImg']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>

</div>

<div class="setGroup"><div style="border-bottom: solid 2px grey;"class="setListItem"><h3>Dev</h3></div>
<br><div class="setListItem"><label for="providers">Show providers:</label> <label class="switch">
<input type="submit" style="display:none;" name="providers" id="providers" value="providers';if (isset($_COOKIE['providers'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (isset($_COOKIE['providers']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>
<br><div class="setListItem"><label for="showtime">Show time:</label> <label class="switch">
<input type="submit" style="display:none;" name="showtime" id="showtime" value="showtime';if (isset($_COOKIE['showtime'])){echo 'On';}else{echo 'Off';}echo '">
<input type="checkbox" class="setCheck" ';
if (isset($_COOKIE['showtime']))
{
    echo 'checked';
}
echo '>
<span class="slider round"></span>
</label></div>
</div>
<div style="height:70px;"></div>
</div></form></nav></div>';