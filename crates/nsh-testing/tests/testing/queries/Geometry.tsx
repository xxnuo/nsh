import { List, mount, Scroll, Text, View } from "nsh";

mount(() => (
  <View style={{ flex: 1 }}>
    <Scroll testID="list" style={{ height: 80, flexShrink: 0 }}>
      <View style={{ height: 600, flexShrink: 0 }}>
        <Text>First row</Text>
        <View style={{ height: 450, flexShrink: 0 }} />
        <Text testID="last">Last row</Text>
      </View>
    </Scroll>
    <List each={Array.from({ length: 100 }, (_, index) => index)} estimatedItemSize={40} style={{ height: 80, flexShrink: 0 }}>
      {(index) => (
        <View testID={`virtual-${index()}`} style={{ height: 40, flexShrink: 0 }}>
          <Text>{`Virtual ${index()}`}</Text>
        </View>
      )}
    </List>
  </View>
));
