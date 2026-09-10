import { SafeAreaView } from 'react-native';

import { Greeting } from './src/Greeting';

export default function App(): React.JSX.Element {
    return <SafeAreaView><Greeting name="Ada" /></SafeAreaView>;
}
