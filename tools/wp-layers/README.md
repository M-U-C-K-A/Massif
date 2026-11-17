# Writing WorldPainter layer files

`MakeLayers.java` writes one `.layer` file of each kind of custom layer with
WorldPainter's own classes, as its Export does, for the tests of
`crates/massif-legacy/tests/data/layers`. It is in WorldPainter's package
`org.pepsoft.worldpainter.layers.plants` to set a plant's settings.

With a WorldPainter checkout built (`WPCore/target/WPCore-*.jar`) and
Java 17 or later:

```sh
cd WorldPainter/WPCore && mvn -o -q dependency:build-classpath -Dmdep.outputFile=/tmp/wpcore.cp
CP="$(cat /tmp/wpcore.cp):$PWD/target/WPCore-2.27.2-modernised.14.jar"
javac -proc:none -cp "$CP" -d /tmp/wp-layers MakeLayers.java
java -Djava.awt.headless=true -cp "/tmp/wp-layers:$CP" org.pepsoft.worldpainter.layers.plants.MakeLayers <folder>
```
