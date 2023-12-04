<?php
  //Get data from $PromoFile
  $promodata = file_get_contents('./Controller/value/data.json');
  $promoobj = json_decode($promodata, true);
$langName = [""];
$langVal = [""];
foreach ($promoobj['lang'][0] as $location) {
  array_push($langVal, $location);
}
foreach ($promoobj['lang'][0] as $name => $value) {
  array_push($langName, $name);
}

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


include './Controller/functions/indexLogic.php';

echo '

<button class="tree-btn" id="tree-btn"><img alt="icntree" src="./View/icon/user.svg" style="width:30px;height:30px;"><p style="font-weight:bold;">'
, $usr , '</p></button>
  <br>
  ';
  include 'settings.php';
?>

<div class="autocomplete">
<div style="margin: 0;
    position: absolute;
    top: 43%;
    left: 50%;
    transform: translate(-50%, -125px);z-index:10;width:clamp(0px, 659px, 100%);text-align:center;">

    <img id="LandSLogo" alt="PriEcoLogo" style="height: 54px;width: 25.65px;"src="./View/img/PriEco.webp?1" />
    <span style="font-size: 50px;font-weight:bold;width:40%;background: -webkit-linear-gradient(#03D781, #3EDCE2);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;">PriEco</span>

<form id="searchForm" class="searchBarMain" method="post" style="margin-top:10px;">
    <button id="searchButton" aria-label="search button" style="border:none;width:8%;height:50px;right:-10px;left:unset;border-radius: 50px 0 0 50px;" class="searchButtonMain searchButton">
      <img src="./View/icon/search.webp" alt="‎" style="width:13px;height:13px;"></button>
      <input list="suggestions" id="searchBox" placeholder="Search privately and ecofriendly" value="" class="searchBox" name="q" size="21" autofocus="" autocomplete="off" style="height: 50px;
width: calc(92% - 10px);border:none;
padding-left: 20px;margin-left:unset;
min-width: 200px;
border-radius: 0 50px 50px 0;
"><button class="delQueryBtn" onclick="delFuc()" type="button" style="width: 20px;height: auto;margin-left: -43px;"><img src="View/icon/cross.svg" style="filter: opacity(0.7);width:100%;height:100%;"></button>
  </form>
  <div class="autoboxMain autocom-box" style="width: calc(90% - 30px);min-width: 200px;position:absolute;margin-left: 50px;margin-top: 0;border-radius: 0 0 20px 20px;box-shadow:unset;"></div>

<div class="shortcuts">
<?php
if(isset($_COOKIE['shortcuts'])){
$shortcuts = explode(',', urldecode($_COOKIE['shortcuts']));
$i =0;
foreach($shortcuts as &$shs){
  ++$i;
  if($i == 1){continue;}
$sh = explode('=',$shs);
echo '<div>

<div>
<div tabindex="0" class="shortcutEditBtn',$i,' shortcutEditBtn">•</div>
<a href="',$sh[1],'" class="shortcutBtn',$i,'" style="text-decoration:none;color:black;">
<div tabindex="0" class="shortcutElemenet" style="background-image: url(/Controller/functions/proxy.php?q=https://judicial-peach-octopus.b-cdn.net/'. parse_url($sh[1])['host']. ');"></div>
</a>
<div class="shortcutEditForm',$i,' shortcutEditForm shortcutForm">
<form method="post">
<input type="hidden" name="shortcutID" value="',$i,'">
<input type="submit" name="shortcutDelete" value="Delete" style="background-color:red;margin-bottom:20px;cursor:pointer;">
</form>
<p><b>Edit</b></p>
<p>',$sh[0],'</p>
<form method="post">
<input type="hidden" name="shortcutID" value="',$i,'">
<input type="text" name="shortcutName" placeholder="Name" value="',$sh[0],'" required>
<input type="text" name="shortcutURL" placeholder="URL" value="',$sh[1],'" required>
<input type="submit" name="shortcutEdit" value="Edit" style="cursor:pointer;">
</form>
</div>
<div>
<p>';
if(strlen($sh[0])>10){echo substr($sh[0], 0, 7).'...';}
else{echo $sh[0];}
echo'</p>
</div>
</div>
</div>

<style>
.shortcutEditBtn',$i,':focus ~ .shortcutEditForm',$i,'{
  display: flex;
}
</style>';

}
}
?>

<div tabindex="0" id="addShortcutBtn" class="shortcutElemenet">+</div>
<div class="addShortcut shortcutForm">
<p><b>Add shortcut</b></p>
<form method="POST">
<input type="text" name="shortcutName" placeholder="Name" required>
<input type="text" name="shortcutURL" placeholder="URL" required>
<input type="submit" name="shortcutSubmit" value="Add" style="cursor:pointer;">
</form>
</div>
  </div>
</div>
</div>

  <div style="height: 98vh;width:auto; background-image: url('/View/img/wbg.webp');background-repeat: no-repeat;
background-position: center;
background-size: cover;"></div>

<div style="width:auto;height:<?php echo ($i-1)/4*111;?>px"></div>

<style>
  @media (max-width: 890px) {
    .autocomplete{
      margin-top: -80px;
    }
    .delQueryBtn {
    top: 97.5px;
    right: 20px;
  }
  .searchButton{
    right:unset !important;
    width: 10% !important;
  }
  }
  .menu-btn{
    position: absolute !important;
    top: 14px !important;
  }
  
  .searchButton::after {
  right: 0px;
  left:unset;
}
</style>