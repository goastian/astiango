<?php
$timezones = DateTimeZone::listIdentifiers();

$timezone = null;
foreach ($timezones as $tz) {
    if (explode('/', $tz)[1] == $OpenWeatherObj['name']) {
        $timezone = $tz;
        break;
    }
}

if ($timezone !== null) {
    date_default_timezone_set($timezone);
}

if ($weatherTrue && $OpenWeatherObj['main']['temp']!=0) {

  echo '<div class="output"style="margin-bottom: 15px;border-radius: 20px;padding: 10px;">

    <div style="width:100%;display:flex;">
      <div style="margin-bottom: 20px;display: flex;justify-content: space-between;border-bottom: solid gray 1px;width:100%;padding: 0 10px;">
        <p style="font-weight: bold;">',$OpenWeatherObj['name'],', ', $OpenWeatherObj['sys']['country'],'</p>
        <p>',date('l') ,'</p>
      </div>
    </div>
    
  <div style="width:100%;display:flex;align-items: center;justify-content: space-between;margin-bottom:10px;">
    <div style="display: flex;align-items: center;">';
    if(!isset($_COOKIE['datasave'])){
      echo '<img style="width:100px;height:100px;"src="/Controller/functions/proxy.php?q=https://openweathermap.org/img/wn/',$OpenWeatherObj['weather'][0]['icon'],'@2x.png">';
    }
    echo'
      <div> 
        <p style="font-size: 36px;" id="temp">';
          if(!isset($_COOKIE['temp'])){echo round($OpenWeatherObj['main']['temp']-273.15, 2), ' °C';} 
          elseif($_COOKIE['temp'] == 'f'){echo round(($OpenWeatherObj['main']['temp']- 273.15) * 9/5 + 32, 2), ' °F';}
          else{echo round($OpenWeatherObj['main']['temp'], 2),' K';} 
        echo'</p>
        <p id="ftemp">Feels: ';
          if(!isset($_COOKIE['temp'])){echo round($OpenWeatherObj['main']['feels_like']-273.15, 2), ' °C';} 
          elseif($_COOKIE['temp'] == 'f'){echo round(($OpenWeatherObj['main']['feels_like']- 273.15) * 9/5 + 32, 2), ' °F';}
          else{echo round($OpenWeatherObj['main']['feels_like'], 2),' K';} 
        echo'</p>';
        /*if(!isset($_COOKIE['DisWid'])){
          echo'<button id="btn-celsius" class="';if($_COOKIE['temp'] == 'c'){echo 'weatherBtnActive';}echo ' weatherCsetting">C</button>
          <button id="btn-fahrenheit" class="';if(isset($_COOKIE['temp']) && $_COOKIE['temp']=='f'){echo 'weatherBtnActive';}echo ' weatherCsetting">F</button>
          <button id="btn-kelvin" class="';if(isset($_COOKIE['temp']) && $_COOKIE['temp']=='k'){echo 'weatherBtnActive';}echo ' weatherCsetting">K</button>';
        }*/
      echo'</div>
    </div>
    <div>
      <p style="font-weight:bold;">',ucfirst($OpenWeatherObj['weather'][0]['main']),'</p>
      <p style="font-size: 12px;margin-bottom: 5px;">',ucfirst($OpenWeatherObj['weather'][0]['description']),'</p>
      <p style="font-size: 12px;">Humidity: ',$OpenWeatherObj['main']['humidity'],'%</p>
      <p style="font-size: 12px;">Pressure: ',$OpenWeatherObj['main']['pressure'],'P</p>
      <p style="font-size: 12px;">Wind: ',round($OpenWeatherObj['wind']['speed']*3.6, 2),'k/h</p>';

      if($timezone !== null){
      echo '<br>
      <p style="font-size: 12px;">☀️Sunrise: ',date('H:i', $OpenWeatherObj['sys']['sunrise']),'</p>
      <p style="font-size: 12px;">🌑Sunset: ',date('H:i', $OpenWeatherObj['sys']['sunset']),'</p>';
      }
    echo '</div>

    <div>
    </div>
    
  </div>';

    $weatherNum = 0;
    $cday = -1;
    foreach($OpenWeatherForecastObj['list'] as &$owfo){
      if($cday != date("d", strtotime($owfo['dt_txt']))){
        $cday = date("d", strtotime($owfo['dt_txt']));



        echo '<input type="radio" id="weatherRadio',$weatherNum,'" name="weatherRadio" style="display:none;"';if($weatherNum==0){echo'checked';}echo'>
        <div class="weatherChart',$weatherNum,' weatherChart">';
        ++$weatherNum;
  foreach($OpenWeatherForecastObj['list'] as &$owfo2)
  {
    if(date("d", strtotime($owfo2['dt_txt'])) == $cday)
  {
    $temperatures[] = $owfo2['main']['temp'];
    $times[] = date("H", strtotime($owfo2['dt_txt']));
  }
}

  $chartPadding = 30;
$pointInterval = 100;

$dataMin = min($temperatures);
$dataMax = max($temperatures);

$chartLeft = $chartPadding;
$chartRight = count($temperatures) * $pointInterval + $chartPadding;
$chartTop = $chartPadding;
$chartBottom = 200 - $chartPadding;

$dataPoints = [];
$pointCount = count($temperatures);
for ($i = 0; $i < $pointCount; $i++) {
    $x = $chartLeft + $pointInterval * $i;
    if($dataMax != $dataMin){$y = $chartBottom - (($temperatures[$i] - $dataMin) / ($dataMax - $dataMin)) * ($chartBottom - $chartTop);}
    else{$y = $chartBottom - ($temperatures[$i] - $dataMin) * ($chartBottom - $chartTop);}
    $dataPoints[] = [$x, $y];
}

$svgWidth = $chartRight;
$svgHeight = 200;

echo '<p><b>',date("l", strtotime($owfo['dt_txt'])),'</b></p><svg width="' , $svgWidth , '" height="' , $svgHeight , '">

<defs>
  <filter id="shadow" x="0%" y="0%" width="140%" height="140%">
  <feDropShadow dx="0" dy="0" stdDeviation="20" flood-color="yellow"/>
  </filter>
</defs>

<polyline points="';
foreach ($dataPoints as $point) {
    echo $point[0] , ',' , $point[1] , ' ';
}
echo '" fill="none" stroke-width="7" stroke="yellow" filter="url(#shadow)" />';


foreach ($dataPoints as $point) {
    echo '<circle cx="' , $point[0] , '" cy="' . $point[1] , '" r="7" fill="yellow" />';
}

$textOffsetY = -15;
foreach ($dataPoints as $index => $point) {
    $textX = $point[0];
    $textY = $point[1] + $textOffsetY;
    if(!isset($_COOKIE['temp'])){$text = round($temperatures[$index]-273.15, 2).'°';}
    elseif($_COOKIE['temp'] == 'f'){$text = round(($temperatures[$index] -  273.15) * 9/5 + 32,2).'°';}
    else{$text = round($temperatures[$index],2).'°';}
    echo '<text x="' , $textX , '" y="' , $textY , '" text-anchor="middle" fill="gray">' , $text , '</text>';
}

$textOffsetY = 25;
foreach ($dataPoints as $index => $point) {
    $textX = $point[0];
    $textY = 180;
    $text = $times[$index];
    echo '<text x="' , $textX , '" y="' , $textY , '" text-anchor="middle" fill="gray">' , $text , '</text>';
}

echo '</svg></div>';
unset($temperatures);
unset($times);
      }
    }
    echo '<div class="weatherForecastList">';

    $weatherNum = 0;
    $cday = -1;
    foreach($OpenWeatherForecastObj['list'] as &$owfo){
      if($cday != date("d", strtotime($owfo['dt_txt']))){
        $cday = date("d", strtotime($owfo['dt_txt']));
        foreach($OpenWeatherForecastObj['list'] as &$owfo2)
        {
          if(date("d", strtotime($owfo2['dt_txt'])) == $cday)
          {
            $temp[] = $owfo2['main']['temp'];
          }
      }
        echo '
        <label for="weatherRadio',$weatherNum,'" class="weatherLabel">
        <p>', date("l", strtotime($owfo['dt_txt'])),'</p>';
        if(!isset($_COOKIE['datasave'])){
        echo'<img style="width:60px;height:60px;"src="/Controller/functions/proxy.php?q=https://openweathermap.org/img/wn/',$owfo['weather'][0]['icon'],'@2x.png">';
        }
        else{
          echo '<div style="height:60px;width:60px;"></div>';
        }
        echo '<div>';
          if(!isset($_COOKIE['temp'])){echo round(max($temp)-273.15, 2).'°';}
          elseif($_COOKIE['temp'] == 'f'){echo round((max($temp) -  273.15) * 9/5 + 32,2).'°';}
          else{echo round(max($temp),2).'°';}
        echo '<p style="opacity:0.7;">';
          if(!isset($_COOKIE['temp'])){echo round(min($temp)-273.15, 2).'°';}
        elseif($_COOKIE['temp'] == 'f'){echo round((min($temp) -  273.15) * 9/5 + 32,2).'°';}
        else{echo round(min($temp),2).'°';}
        echo'</p></div>
      </label>
      <style>
      #weatherRadio',$weatherNum,':checked~.weatherChart',$weatherNum,' {
        visibility: visible;
        height:220px;
      }
      </style>
      ';
      ++$weatherNum;
      unset($temp);
      }
    }
echo '</div>
  <div style="float: right;">
    <a href ="https://openweathermap.org/" style="font-size: 12px;font-weight: bold;opacity:0.5;"><p>Data from OpenWeatherMap</p></a>
    </div>
  </div>';
}
/*
if(!isset($_COOKIE['DisWid'])){
  echo "
  <script>
  const btnCelsius = document.getElementById('btn-celsius');
  const btnFahrenheit = document.getElementById('btn-fahrenheit');
  const btnKelvin = document.getElementById('btn-kelvin');
  const temperature = document.getElementById('temp');
  const Feeltemperature = document.getElementById('ftemp');
  
  btnCelsius.addEventListener('click', function() {
    btnCelsius.classList.add('weatherBtnActive');
    btnFahrenheit.classList.remove('weatherBtnActive');
    btnKelvin.classList.remove('weatherBtnActive');
  
    if(temperature.textContent.slice(-1) == 'K'){temperature.textContent=(Math.round((parseFloat(temperature.textContent) - 273.15) * 100) / 100)+' °C';}
    else if(temperature.textContent.slice(-1) == 'F'){temperature.textContent=(Math.round(((parseFloat(temperature.textContent) - 32) * 5/9) * 100) / 100)+' °C';}
  
    if(Feeltemperature.textContent.slice(-1) == 'K'){Feeltemperature.textContent='Feels: '+(Math.round((parseFloat(Feeltemperature.textContent.match(/[0-9.]+/)[0]) - 273.15) * 100) / 100)+' °C';}
    else if(Feeltemperature.textContent.slice(-1) == 'F'){Feeltemperature.textContent='Feels: '+(Math.round(((parseFloat(Feeltemperature.textContent.match(/[0-9.]+/)[0]) - 32) * 5/9) * 100) / 100)+' °C';}
  
    document.cookie = 'temp=c; expires=Thu, 01 Jan 1970 00:00:00 UTC; path=/;';
  });
  
  btnFahrenheit.addEventListener('click', function() {
    btnCelsius.classList.remove('weatherBtnActive');
    btnFahrenheit.classList.add('weatherBtnActive');
    btnKelvin.classList.remove('weatherBtnActive');
  
    if(temperature.textContent.slice(-1) == 'C'){temperature.textContent=(Math.round(((parseFloat(temperature.textContent) * 9/5) + 32) * 100) / 100)+' °F';}
    else if(temperature.textContent.slice(-1) == 'K'){temperature.textContent=(Math.round(((parseFloat(temperature.textContent) - 273.15) * 9/5 + 32) * 100) / 100)+' °F';}
  
    if(Feeltemperature.textContent.slice(-1) == 'C'){Feeltemperature.textContent='Feels: '+(Math.round(((parseFloat(Feeltemperature.textContent.match(/[0-9.]+/)[0]) * 9/5) + 32) * 100) / 100)+' °F';}
    else if(Feeltemperature.textContent.slice(-1) == 'K'){Feeltemperature.textContent='Feels: '+(Math.round(((parseFloat(Feeltemperature.textContent.match(/[0-9.]+/)[0]) - 273.15) * 9/5 + 32) * 100) / 100)+' °F';}
  
    document.cookie = 'temp=f';
  });
  
  btnKelvin.addEventListener('click', function() {
    btnCelsius.classList.remove('weatherBtnActive');
    btnFahrenheit.classList.remove('weatherBtnActive');
    btnKelvin.classList.add('weatherBtnActive');
  
    if(temperature.textContent.slice(-1) == 'C'){temperature.textContent=(Math.round((parseFloat(temperature.textContent) + 273.15) * 100) / 100)+' K';}
    else if(temperature.textContent.slice(-1) == 'F'){temperature.textContent=(Math.round(((parseFloat(temperature.textContent) - 32) * 5/9 + 273.15) * 100) / 100)+' K';}
  
    if(Feeltemperature.textContent.slice(-1) == 'C'){Feeltemperature.textContent='Feels: '+(Math.round((parseFloat(Feeltemperature.textContent.match(/[0-9.]+/)[0]) + 273.15) * 100) / 100)+' K';}
    else if(Feeltemperature.textContent.slice(-1) == 'F'){Feeltemperature.textContent='Feels: '+(Math.round(((parseFloat(Feeltemperature.textContent.match(/[0-9.]+/)[0]) - 32) * 5/9 + 273.15) * 100) / 100)+' K';}
  
    document.cookie = 'temp=k';
  });
  
  </script>";
}*/
?>