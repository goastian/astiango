<?php

echo '<div class="imgtools">
<form method="post" action="">
<div style="overflow-x:auto;overflow-y:hidden;display:flex;">
<select name="imgtoolsSize" class="imgtoolsOption">
<option selected hidden value="' . $imgsize . '">';
 switch ($imgsize) {
     case 'small':
         echo 'Small';
         break;
     case 'medium':
         echo 'Medium';
         break;
     case 'large':
         echo 'Large';
         break;
     default:
         echo 'Size';
         break;
 }
 echo '
  </option>
  <option value="all">All</option>
  <option value="small">Small</option>
  <option value="medium">Medium</option>
  <option value="large">Large</option>
</select>

<select name="imgtoolsColor" class="imgtoolsOption">
<option selected hidden value="' . $imgcolor . '">';
 switch ($imgcolor) {
     case 'monochrome':
         echo 'Black and white';
         break;
     case 'coloronly':
         echo 'Color only';
         break;
     case 'red':
         echo '🔴 Red';
         break;
     case 'orange':
         echo '🟠 Orange';
         break;
     case 'yellow':
         echo '🟡 Yellow';
         break;
     case 'green':
         echo '🟢 Green';
         break;
     case 'teal':
         echo '🔵 Teal';
         break;
     case 'blue':
         echo '🔵 Blue';
         break;
     case 'purple':
         echo '🟣 Purple';
         break;
     case 'pink':
         echo '🔴 Pink';
         break;
     case 'white':
         echo '⚪ White';
         break;
     case 'grey':
         echo '⚫ Gray';
         break;
     case 'black':
         echo '⚫ Black';
         break;
     case 'brown':
         echo '🟤 Brown';
         break;
     default:
         echo 'Color';
         break;
 }
 echo '
 </option>
 <option value="all">All</option>
 <option value="monochrome">Black and white</option>
 <option value="coloronly">Color only</option>
 <option value="red">🔴 Red</option>
 <option value="orange">🟠 Orange</option>
 <option value="yellow">🟡 Yellow</option>
 <option value="green">🟢 Green</option>
 <option value="teal">🔵 Teal</option>
 <option value="blue">🔵 Blue</option>
 <option value="purple">🟣 Purple</option>
 <option value="pink">🔴 Pink</option>
 <option value="white">⚪ White</option>
 <option value="gray">⚫ Gray</option>
 <option value="black">⚫ Black</option>
 <option value="brown">🟤 Brown</option>
 </select>
 <select name="imgtoolsType" class="imgtoolsOption">
 <option selected hidden value="' . $imgtype . '">';
 switch ($imgtype) {
     case 'photo':
         echo 'Photograph';
         break;
     case 'animatedgif':
         echo 'Gif';
         break;
     case 'transparent':
         echo 'Transparent';
         break;
     default:
         echo 'Type';
         break;
 }
 echo '
 <option value="all">All</option>
 <option value="photo">Photograph</option>
 <option value="animatedgif">Gif</option>
 <option value="transparent">Transparent</option>
 </select>

 <select name="imgtoolsTime" class="imgtoolsOption">
 <option selected hidden value="' . $imgtime . '">';
 switch ($imgtime) {
     case 'day':
         echo 'Past 24 hours';
         break;
     case 'week':
         echo 'Past week';
         break;
     case 'month':
         echo 'Past month';
         break;
     default:
         echo 'Time';
         break;
 }
 echo '
 <option value="all">All</option>
 <option value="day">Past 24 hours</option>
 <option value="week">Past week</option>
 <option value="month">Past month</option>
 </select>

 <select name="imgtoolsRights" class="imgtoolsOption">
 <option selected hidden value="' . $imgright . '">';
 switch ($imgright) {
     case 'public':
         echo 'Public domain';
         break;
     case 'modifycommercially':
         echo 'Free to modify, share and use commercially ';
         break;
     case 'sharecommercially':
         echo 'Free to share and use commercially ';
         break;
     case 'modify':
         echo 'Free to modify, share and use noncommercially';
         break;
     case 'share':
         echo 'Free to share and use noncommercially';
         break;
     default:
         echo 'License';
         break;
 }
 echo '
 <option value="all">All</option>
 <option value="share">Free to share and use</option>
 <option value="sharecommercially">Free to share and use commercially</option>
 <option value="modify">Free to modify, share and use</option>
 <option value="modifycommercially">Free to modify, share and use commercially</option>
 <option value="public">Public domain</option>   
 </select>
 </div>
 <input class="imgtoolsOption"type="submit" name="imgtoolsSave" value="Save" style="
 float: right;
background-color: var(--topColor);
color: white;
border: none;">

 </form>
 <form method="post" action="">
 <input class="imgtoolsOption"type="submit"style="margin-top: 5px;margin-bottom: 10px;" name="pixabayimg" value="Pixabay">
 </form>
 </div>
 <a href="https://www.google.com/search?q='.$purl.'&tbm=isch"';
 if (isset($_COOKIE['new'])) {
    echo 'target="_blank"';
}
echo '><img src="View/img/google.svg" style="
 width: 45px;
 height:45px;
 margin-left: 2vw;
 background-color: #0001;
 padding: 5px;
 border-radius: 20px;
 box-shadow: 0 0 5px 5px #0002;"></a>
 ';