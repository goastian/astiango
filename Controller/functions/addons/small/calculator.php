

    <div class="calculator output" style="border-radius:20px;padding:10px;margin-bottom:15px;">
        <div class="display">
            <input id="screen" type="text" placeholder=":)">
        </div>
        <input type="radio" id="calcFx" name="calcGroup" style="display:none;">

          <div class="halfCalculator" id="halfCalculator2">

            <div>
                <button onclick="fact()">x!</button>
                <button id="calcBtn" >%</button>
                <button id="ce" class="calcMobile" onclick="backspc()" style="color:white;background-color:orangered;">CE</button>
                <button id="ac" class="calcMobile" onclick="screen.value=''" style="color:white;background-color:red;">AC</button>
            </div>

            <div >
                <button onclick="sin()">sin</button>
                <button onclick="pi()">π</button>

            </div>

            <div >
                <button onclick="cos()">cos</button>
                <button onclick="log()">log</button>
            </div>

            <div >
                <button onclick="tan()">tan</button>
                <button onclick="sqrt()">√</button>

            </div>

            <div >
                <button onclick="e()">e</button>
                <button onclick="pow()">x <span style="position: relative; bottom: .4em; right: .1em;">2</span>
                </button>
            </div>
      </div>
      <input type="radio" id="calc123" name="calcGroup" style="display:none;">

      <div class="halfCalculator" id="halfCalculator1">
            <div>
          <button id="calcBtn" >(</button>
                <button id="calcBtn" >)</button>
                <button id="ce" onclick="backspc()" style="color:white;background-color:orangered;">CE</button>
                <button id="ac" onclick="screen.value=''" style="color:white;background-color:red;">AC</button>
  </div>
  <div>
  <button id="calcBtn" >7</button>
                <button id="calcBtn" >8</button>
                <button id="calcBtn" >9</button>
                <button id="calcBtn" >÷</button>
  </div>
  <div>
  <button id="calcBtn" >4</button>
                <button id="calcBtn" >5</button>
                <button id="calcBtn" >6</button>
                <button id="calcBtn" >×</button>
  </div>
  <div>
  <button id="calcBtn" >1</button>
                <button id="calcBtn" >2</button>
                <button id="calcBtn" >3</button>
                <button id="calcBtn" >-</button></div>
                <div>
                <button id="calcBtn" >0</button>
                <button id="calcBtn" >.</button>
                <button id="eval" onclick="screen.value=eval(screen.value).toFixed(2)" style="color:white;background-color:#1ece56;">=</button>
                <button id="calcBtn" >+</button>
  </div>
          </div>
          <div class="calcMobile"> 
            <label for="calc123" style="border-radius: 20px 0 0 20px;">123</label>
            <label for="calcFx" style="border-radius: 0 20px 20px 0;">Fx</label>
          </div>
    </div>


<script>
  var screen = document.querySelector('#screen');
    var btn = document.querySelectorAll('#calcBtn');

    screen.addEventListener('keydown', function(event) {
if (event.key === 'Enter') {
  screen.value=eval(screen.value)
}
else if(event.key == 'Escape'){
screen.value="";
}
});

    for (item of btn) {
        item.addEventListener('click', (e) => {
            btntext = e.target.innerText;

            if (btntext == '×') {
                btntext = '*';
            }

            if (btntext == '÷') {
                btntext = '/';
            }
            screen.value += btntext;
        });
    }

    function sin() {
        screen.value = Math.sin(screen.value).toFixed(2);
    }

    function cos() {
        screen.value = Math.cos(screen.value).toFixed(2);
    }

    function tan() {
        screen.value = Math.tan(screen.value).toFixed(2);
    }

    function pow() {
        screen.value = Math.pow(screen.value, 2).toFixed(2);
    }

    function sqrt() {
        screen.value = Math.sqrt(screen.value, 2).toFixed(2);
    }

    function log() {
        screen.value = Math.log(screen.value).toFixed(2);
    }

    function pi() {
        screen.value =screen.value + 3.14159265359;
    }

    function e() {
        screen.value =screen.value + 2.71828182846;
    }

    function fact() {
        var i, num, f;
        f = 1
        num = screen.value;
        for (i = 1; i <= num; i++) {
            f = f * i;
        }

        i = i - 1;

        screen.value = f.toFixed(2);
    }

    function backspc() {
        screen.value = screen.value.substr(0, screen.value.length - 1);
    }
</script>