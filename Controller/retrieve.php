<!--  SPDX-FileCopyrightText: 2022, 2022-2022 Roman  Láncoš <jojoyou@jojoyou.org> -->
<!-- -->
<!--  SPDX-License-Identifier: AGPL-3.0-or-later -->

<?php
$apiPurl = urlencode($purl);

$weatherTrue=false;
$weatherWords=['طقس','توقعات','درجة الحرارة','време','прогноза','температура','temps','pronòstic','temperatura','pocasie','počasie','predpoved','teplota','vejr','prognose','temperatur','Wetter','Vorhersage','Temperatur','καιρός','πρόγνωση','θερμοκρασία','weather','forecast','temperature','tiempo','pronóstico','temperatura','ilm','prognoos','temperatuur','sää','ennuste','lämpötila','météo','prévisions','température','vrijeme','prognoza','temperatura','időjárás','előrejelzés','hőmérséklet','cuaca','prakiraan','suhu','vedur','spá','hitastig','meteo','previsioni','temperatura','מזג_אוויר','תחזית','טמפרטורה','天気','予報','気温','날씨','예보','기온','orai','prognozė','temperatūra','laiks','prognoze','temperatūra','weer','voorspelling','temperatuur','vær','varsel','temperatur','pogoda','prognoza','temperatura','tempo','previsão','temperatura','vremea','prognoza','temperatura','погода','прогноз','температура','pocasie','počasie','predpoved','teplota','vreme','napoved','temperatura','vreme','prognoza','temperatura','väder','prognos','temperatur','hava_durumu','tahmin','sıcaklık','cet','ct',];
foreach ($weatherWords as $weatherWord) {
    if(strpos(strtolower($purl), $weatherWord) !== false){
        $weatherTrue = true;
        $OpenWeatherLoc = mb_ereg_replace($weatherWord, '', $purl);
        break;
    }
  }

if($weatherTrue){$OpenWeatherLoc = str_replace(' ','+',urldecode(preg_replace('/\s+/', '+', trim($OpenWeatherLoc))));}

$newsdate = date('m/d/Y');
$newsdate = strtotime($newsdate);
$newsdate = strtotime("-7 day", $newsdate);

if (strpos($purl, 'def') !== false || strpos($purl, 'mean') !== false) {$defWords = str_replace(' ', '%20', preg_replace('/\b\w*def\w*\b/', '', preg_replace('/\b\w*mean\w*\b/', '', $purl)));}

$tmp = $lang;
if($tmp==null or $tmp == 'all'){
    $tmp = 'en';
}
if(!$dev) {
    //API keys ($_ENV variables loaded in database.php file) 
    $Googlefile = 'https://www.googleapis.com/customsearch/v1?key='.$_ENV['GOOGLE_API_KEY'].'&cx='.$_ENV['GOOGLE_CX_KEY'].'&hl='.$lang.'&gl='.$loc.'&dateRestrict='.$date.'&safe='.$safe.'&q='.$apiPurl;
    $MojeekFile = 'https://www.mojeek.com/search?api_key='.$_ENV['MOJEEK_API_KEY'].'&lb='.$lang.'&lbb=50'.'&rb='.$loc.'&rbb=50&fmt=json&q='.$apiPurl;
    
    if(isset($OpenWeatherLoc)){$OpenWeatherFile = 'https://api.openweathermap.org/data/2.5/weather?appid='.$_ENV['OPENWEATHER_API_KEY'].'&q='.$OpenWeatherLoc;
    $OpenWeatherForecastFile = 'https://api.openweathermap.org/data/2.5/forecast?appid='.$_ENV['OPENWEATHER_API_KEY'].'&q='.$OpenWeatherLoc;}
    if(isset($defWords)){$WordnikFile = 'https://api.wordnik.com/v4/word.json/'.$defWords.'/definitions?limit=5&includeRelated=false&useCanonical=false&includeTags=false&api_key='.$_ENV['WORDNIK_API_KEY'];}
    else{$WordnikFile ='';}

    $PixabayFile = 'https://pixabay.com/api/?key='.$_ENV['PIXABAY_API_KEY'].'&per_page=200&q='.$apiPurl;

    $NewsFile = 'https://newsapi.org/v2/everything?apiKey='.$_ENV['NEWS_API_KEY'].'&from='.date('Y-m-d', $newsdate).'&sortBy=popularity&language='.$tmp.'&q='.$apiPurl;
    $YoutubeFile = 'https://www.googleapis.com/youtube/v3/search?part=snippet&type=video&maxResults=50&key='.$_ENV['YOUTUBE_API_KEY'].'&q='.$apiPurl;
}
else{
    //Null API keys    
    $Googlefile = './Controller/dev/google.json';
    $MojeekFile = './Controller/dev/mojeek.json';
    
    $OpenWeatherFile='./Controller/dev/openweather.json';
    $OpenWeatherForecastFile = './Controller/dev/openweatherForecast.json';
    $RedditFile = './Controller/dev/reddit.json';
    $DdgFile = './Controller/dev/ddg.json';

    $PixabayFile = './Controller/dev/pixabay.json';
    $YoutubeFile = './Controller/dev/yt.json';
    $NewsFile = './Controller/dev/news.json';
}

$PromoURL = './Controller/value/data.json';