<?php
echo'<link rel="stylesheet" href="./css/style.css?v=',$cssver,'">
<link rel="stylesheet" href="./css/mobileStyle.css?v=',$cssver,'">';

if(!isset($_COOKIE['mode'])){
  //Auto mode
  echo '<style>
@media (prefers-color-scheme: light) { 
'; echo file_get_contents('./css/light.css');echo'
}
@media (prefers-color-scheme: dark) {  
  '; echo file_get_contents('./css/dark.css');echo'
}
</style>
';
}
elseif($_COOKIE['mode'] == 1){
//Light mode
    echo '<link rel="stylesheet" href="./css/light.css?v=',$cssver,'">';
}
elseif($_COOKIE['mode'] == 2){
  //Dark mode
    echo '<link rel="stylesheet" href="./css/dark.css?v=',$cssver,'">';    
}
else{
  //Custom mode
      echo '<style>', $_COOKIE['theme'], '</style>';
}
?>
