<?php

echo '<div class="imgtools">
<form method="post" action="">
<div style="overflow-x:auto;overflow-y:hidden;display:flex;">
<div style="display: flex;flex-direction: row;flex-wrap: nowrap;align-items: center;">
<p style="margin-right: 10px;">Price:</p>
<input value="',$_GET['shopMin'],'" name="shopPriceMin" type="number" min="0" placeholder="Min" style="width: 100px;
border: solid 1px gray;
color: #6a6a6a;padding: 5px;
background-color: #00000007;
border-radius: 20px;">
<input value="',$_GET['shopMax'],'" name="shopPriceMax" type="number" min="0" placeholder="Max"style="width: 100px;
border: solid 1px gray;
color: #6a6a6a;padding: 5px;
background-color: #00000007;
border-radius: 20px;">
</div>
 </div>
 <input class="imgtoolsOption"type="submit" name="shopToolsSave" value="Save" style="
 float: right;
background-color: var(--topColor);
color: white;
border: none;">

 </form>
 </div>
 ';