<?php
 if (preg_match('/\bip\b/i', $purl) && !str_starts_with($_SERVER['REMOTE_ADDR'], '192.') && !str_starts_with($_SERVER['REMOTE_ADDR'], '127.')) {
    echo '<div class="output" style="margin-bottom: 15px;border-radius: 20px;padding: 10px;">
      <p style="text-align:center;border-bottom:solid gray 1px;">Your Public IP Address is: <b>',$_SERVER['REMOTE_ADDR'],'</b></p><br>
      <div style="display:flex;justify-content: space-between;flex-wrap: wrap;">
      <div>
      <p><b>Info</b></p>
     <p class="ipInfo">Country: ',$ipObj['country'],'</p>
     <p class="ipInfo">Code: ',$ipObj['code'],'</p>
     <p class="ipInfo">Language: ',$ipObj['lang'],'</p>     
     </div>
     </div>
    </div>';
}