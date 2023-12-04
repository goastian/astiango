<?php
function pHigh($purl){
    $purlLow = strtolower($purl);
    $privateOpen = [
        'twitter;Mastodon;mastodon.webp;https://joinmastodon.org/;C;639',
        'instagram;Pixelfed;pixelfed.webp;https://pixelfed.org/',
        'instagram,twitter;Momenel;momenel.svg;https://www.momenel.com/',

        'windows,macos,mac os;Ubuntu;ubuntu.webp;https://ubuntu.com/',
        'windows,macos,mac os;VanillaOS;vanillaos.svg;https://vanillaos.org',

        'mail,email,gmail,outlook;Proton Mail;protonmail.webp;https://proton.me/mail/;A;491',
        'mail,email,gmail,outlook;Skiff Mail;skiffmail.webp;https://skiff.com/mail/',

        'google docs,notion,evernote;Skiff Pages;skiffPage.webp;https://skiff.com/pages',
        
        'chrome,google chrome,opera,edge,chromium,safari,browser;Firefox;firefox.webp;https://www.mozilla.org/en-US/firefox/new/;B;188',
        'chrome,google chrome,opera,edge,chromium,safari,browser;Brave;brave.webp;https://brave.com/;B;1487',
        'chrome,google chrome,opera,edge,chromium,safari,browser;Vivaldi;vivaldi.webp;https://vivaldi.com/;B;2371',

        'chat,messenger;Signal;signal.webp;https://www.signal.org/',
        'chat,messenger;Session;session.webp;https://getsession.org/;B;3015',
        'chat,messenger;Simplex;simplex.webp;https://simplex.chat/',
        'chat,messenger,discord;Revolt;revolt.webp;https://revolt.chat/'

        ];
    $load = false;
    $privateAltOut='<p class="sectionTitle">⚡ Alternatives</p>
    <div class="output" style="display: flex;padding: 15px 0 0 0;flex-wrap: wrap;flex-direction: row;justify-content: center;margin-bottom:15px;border-radius:var(--result-curve);">';

    foreach($privateOpen as &$pO){
        $tmp = explode(';',$pO);
        if(in_array($purlLow, explode(',',$tmp[0]))){
            $load = true;
            $privateAltOut .= '<div style="width: 120px;border-radius: 20px;text-align: center;cursor: pointer;display: flex;flex-direction: column;">
              <a style="padding-bottom:0;margin-bottom:10px;"href="'.$tmp[3].'"';if (isset($_COOKIE['new'])) {$privateAltOut .='target="_blank"';}$privateAltOut .='>';
              if(!isset($_COOKIE['datasave'])){
                $privateAltOut .= '<img src="View/img/privateAlt/'.$tmp[2].'" style="width: 50px;height: 50px;">';
              }
              $privateAltOut .='<p>'.$tmp[1].'</p>
              </a>
              
              <a style="padding-bottom:10px;"href="https://tosdr.org/en/service/'.$tmp['5'].'"';if (isset($_COOKIE['new'])) {$privateAltOut .='target="_blank"';}$privateAltOut .='>
              <p style="background-color:';
              switch($tmp[4]){
                case 'A':
                    $privateAltOut .= '#198754';
                    break;
                case 'B':
                    $privateAltOut .= '#79b752';
                    break;
                case 'C':
                    $privateAltOut .= '#ffc107';
                    break;
                case 'D':
                    $privateAltOut .= '#d66f2c';
                    break;
                case 'E':
                    $privateAltOut .= '#dc3545';
                    break;
            }
              $privateAltOut .= ';margin:0 2.5px;color:white;font-weight:bold;border-radius:var(--result-curve);">'.$tmp[4].'</p>
              </a>
            </div>';
        }
    }
    $privateAltOut .= '</div>';
    if($load){return $privateAltOut;}
    else{return '';}
}