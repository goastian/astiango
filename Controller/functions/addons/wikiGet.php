<?php
include '../../simple_html_dom.php';

 $pageUrl = 'https://'.$_GET['lang'].'.wikipedia.org/wiki/'.$_GET['q'];
 $html = file_get_html($pageUrl); 
 if(gettype($html) != 'object'){return;}
 
 $infobox = $html->find('.infobox', 0);
 $infoboxData = [];
 
 if ($infobox) {
     $images = $infobox->find('img');
     foreach ($images as $image) {
         $imageUrl = $image->src;
         if($imageUrl != '//upload.wikimedia.org/wikipedia/en/thumb/8/8a/OOjs_UI_icon_edit-ltr-progressive.svg/10px-OOjs_UI_icon_edit-ltr-progressive.svg.png'){
         $infoboxData['images'][] = $imageUrl;
         if($ansImg == ''){$ansImg = $imageUrl;}
         }
     }
 
$infobox = $html->find('.infobox', 0);
foreach ($infobox->find('tr') as $row) {
 $cells = $row->find('th, td');

 if (count($cells) == 2) {
     $key = trim($cells[0]->plaintext);
     $value = trim(html_entity_decode($cells[1]->plaintext));
    
    if($key == 'Website'){ $value = preg_replace('/\s+/u', ' ', explode(' ',$value)[0]);}
     $infoboxData[$key] = $value;
 }
}

 }
 $infoboxData['title'] = substr($html->find('title', 0)->plaintext, 0, strpos($html->find('title', 0)->plaintext, ' - '));;

 $content = $html->find('#mw-content-text', 0);
 $wikiTxt = '';
 if ($content) {
     $contentParagraphs = $content->find('h2, p');
     foreach ($contentParagraphs as $paragraph) {
        if(strpos($paragraph, '<h2>') !== false){break;}
        $wikiTxt .= preg_replace('/\[[^\]]*\]/', '',html_entity_decode($paragraph->plaintext));
     }
 }    
 $html->clear();
echo json_encode($infoboxData);
echo '--]|[--',$wikiTxt;