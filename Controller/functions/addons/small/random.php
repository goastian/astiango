<?php
echo '<div class="output" style="margin-bottom: 15px;border-radius: 20px;padding: 10px;">
<b><p id="rnNum" style="text-align:center;font-size:32px;">',rand(1, 10),'</p></b>';

if(!isset($_COOKIE['DisWid'])){
    echo '
    <div style="display:flex;border-top:solid gray 1px;">
    <input class="rnInp" type="number" placeholder="From" id="from" min="0" max="1000000000" step="1">
    
    <input class="rnInp" type="number" placeholder="To" id="to" min="0" max="1000000000" step="1">
    
    <button class="rnInp" style="width:30%;"onclick="generateRandomNumber()">🎲</button>
        </div>
    <script>
      function generateRandomNumber() {
        var from = parseInt(document.getElementById("from").value);
        var to = parseInt(document.getElementById("to").value);
        
        if (isNaN(from) || isNaN(to)) {
          document.getElementById("rnNum").innerHTML = "Invalid input";
        } else if (from >= to) {
          document.getElementById("rnNum").innerHTML = "From value must be less than To value";
        } else {
          var randomNumber = Math.floor(Math.random() * (to - from + 1)) + from;
          document.getElementById("rnNum").innerHTML = randomNumber;
        }
      }
    </script>';
}
echo '</div>';